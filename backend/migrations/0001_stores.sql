CREATE TABLE stores (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    name VARCHAR(200) NOT NULL CHECK (length(trim(name)) > 0),
    business_status VARCHAR(6) NOT NULL CHECK (business_status IN ('OPEN','CLOSED')),
    business_contact VARCHAR(200) NOT NULL,
    manager_user_id UUID,
    manager_display_name VARCHAR(100),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id,organization_id),
    UNIQUE (tenant_id,id,organization_id),
    CHECK ((manager_user_id IS NULL) = (manager_display_name IS NULL))
);
CREATE INDEX stores_tenant_owner ON stores(tenant_id,owner_user_id,organization_id,id);

CREATE TABLE store_staff (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    store_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    user_id UUID NOT NULL,
    display_name VARCHAR(100) NOT NULL CHECK (length(trim(display_name)) > 0),
    created_by_user_id UUID NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    removed_at TIMESTAMPTZ,
    FOREIGN KEY (tenant_id,store_id,organization_id) REFERENCES stores(tenant_id,id,organization_id),
    CHECK (owner_user_id=user_id)
);
CREATE UNIQUE INDEX store_staff_active_member_key ON store_staff(tenant_id,store_id,user_id) WHERE removed_at IS NULL;
CREATE INDEX store_staff_scope ON store_staff(tenant_id,organization_id,owner_user_id,store_id,id) WHERE removed_at IS NULL;

CREATE TABLE store_daily_reports (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    store_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    business_date DATE NOT NULL CHECK (business_date BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
    amount NUMERIC(18,2) NOT NULL CHECK (amount >= 0 AND amount <> 'NaN'::numeric),
    note TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 8192),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id,store_id,organization_id) REFERENCES stores(tenant_id,id,organization_id),
    CONSTRAINT store_daily_reports_day_key UNIQUE (tenant_id,store_id,business_date)
);
CREATE INDEX store_daily_reports_scope ON store_daily_reports(tenant_id,organization_id,owner_user_id,business_date,store_id,id);

CREATE TABLE store_audit_events (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    actor_user_id UUID NOT NULL,
    resource_code VARCHAR(32) NOT NULL CHECK (resource_code IN ('store','store-staff','store-daily-report')),
    action_code VARCHAR(16) NOT NULL CHECK (action_code IN ('create','update','assign','unassign')),
    entity_id UUID NOT NULL,
    request_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX store_audit_events_scope ON store_audit_events(tenant_id,organization_id,created_at,id);

CREATE FUNCTION stores_identity_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.organization_id,NEW.owner_user_id,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.organization_id,OLD.owner_user_id,OLD.created_at)
       OR NEW.version <> OLD.version + 1 THEN
        RAISE EXCEPTION 'store identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER stores_identity_immutable BEFORE UPDATE ON stores FOR EACH ROW EXECUTE FUNCTION stores_identity_immutable();

CREATE FUNCTION store_staff_identity_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.store_id,NEW.organization_id,NEW.owner_user_id,NEW.user_id,NEW.display_name,NEW.created_by_user_id,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.store_id,OLD.organization_id,OLD.owner_user_id,OLD.user_id,OLD.display_name,OLD.created_by_user_id,OLD.created_at)
       OR NEW.version <> OLD.version + 1 OR OLD.removed_at IS NOT NULL THEN
        RAISE EXCEPTION 'staff identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER store_staff_identity_immutable BEFORE UPDATE ON store_staff FOR EACH ROW EXECUTE FUNCTION store_staff_identity_immutable();

CREATE FUNCTION store_daily_report_identity_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.store_id,NEW.organization_id,NEW.owner_user_id,NEW.business_date,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.store_id,OLD.organization_id,OLD.owner_user_id,OLD.business_date,OLD.created_at)
       OR NEW.version <> OLD.version + 1 THEN
        RAISE EXCEPTION 'daily report identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER store_daily_report_identity_immutable BEFORE UPDATE ON store_daily_reports FOR EACH ROW EXECUTE FUNCTION store_daily_report_identity_immutable();

CREATE FUNCTION store_audit_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'store audit events are append only' USING ERRCODE='23514';
END
$$;
CREATE TRIGGER store_audit_append_only BEFORE UPDATE OR DELETE ON store_audit_events FOR EACH ROW EXECUTE FUNCTION store_audit_append_only();

-- This migration records the expected catalog once, atomically with schema creation.
-- Restart/readiness compares against it and never blesses a changed live catalog.
CREATE FUNCTION stores_schema_fingerprint() RETURNS TEXT LANGUAGE sql STABLE AS $$
    SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(item,E'\n' ORDER BY item),'')) FROM (
        SELECT 'column|'||c.relname||'|'||a.attnum||'|'||a.attname||'|'||pg_catalog.format_type(a.atttypid,a.atttypmod)||'|'||a.attnotnull||'|'||COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),'') AS item
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
        LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
        WHERE n.nspname='public' AND c.relname IN ('stores','store_staff','store_daily_reports','store_audit_events','_stores_schema_contract','_sqlx_migrations')
        UNION ALL
        SELECT 'constraint|'||c.relname||'|'||k.conname||'|'||pg_catalog.pg_get_constraintdef(k.oid,true)
        FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND c.relname IN ('stores','store_staff','store_daily_reports','store_audit_events','_stores_schema_contract','_sqlx_migrations')
        UNION ALL
        SELECT 'index|'||tablename||'|'||indexname||'|'||indexdef FROM pg_catalog.pg_indexes
        WHERE schemaname='public' AND tablename IN ('stores','store_staff','store_daily_reports','store_audit_events','_stores_schema_contract','_sqlx_migrations')
        UNION ALL
        SELECT 'trigger|'||c.relname||'|'||t.tgname||'|'||t.tgenabled::text||'|'||pg_catalog.pg_get_triggerdef(t.oid,true)
        FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND NOT t.tgisinternal AND c.relname IN ('stores','store_staff','store_daily_reports','store_audit_events','_stores_schema_contract','_sqlx_migrations')
        UNION ALL
        SELECT 'function|'||p.proname||'|'||pg_catalog.pg_get_functiondef(p.oid)
        FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
        WHERE n.nspname='public' AND p.proname IN ('stores_identity_immutable','store_staff_identity_immutable','store_daily_report_identity_immutable','store_audit_append_only','stores_schema_fingerprint')
        UNION ALL
        SELECT 'relation|'||c.relname||'|'||c.relkind::text||'|'||c.relrowsecurity::text||'|'||c.relforcerowsecurity::text
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'
        UNION ALL
        SELECT 'routine|'||p.proname||'|'||p.prokind::text||'|'||pg_catalog.pg_get_function_identity_arguments(p.oid)
        FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'
        UNION ALL
        SELECT 'type|'||t.typname||'|'||t.typtype::text
        FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public'
    ) contracts
$$;
CREATE TABLE _stores_schema_contract (singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton), fingerprint TEXT NOT NULL);
INSERT INTO _stores_schema_contract(singleton,fingerprint) VALUES(true,stores_schema_fingerprint());

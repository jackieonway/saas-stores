-- Fresh installation baseline. Existing installations must be recreated; no historical adoption.
-- PostgreSQL 17+, application-generated UUIDv7, SSO-only console.
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;

CREATE FUNCTION public.store_audit_append_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    RAISE EXCEPTION 'store audit events are append only' USING ERRCODE='23514';
END
$$;

CREATE FUNCTION public.store_daily_report_identity_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.store_id,NEW.organization_id,NEW.owner_user_id,NEW.business_date,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.store_id,OLD.organization_id,OLD.owner_user_id,OLD.business_date,OLD.created_at)
       OR NEW.version <> OLD.version + 1 THEN
        RAISE EXCEPTION 'daily report identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE FUNCTION public.store_staff_identity_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.store_id,NEW.organization_id,NEW.owner_user_id,NEW.user_id,NEW.display_name,NEW.created_by_user_id,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.store_id,OLD.organization_id,OLD.owner_user_id,OLD.user_id,OLD.display_name,OLD.created_by_user_id,OLD.created_at)
       OR NEW.version <> OLD.version + 1 OR OLD.removed_at IS NOT NULL THEN
        RAISE EXCEPTION 'staff identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE FUNCTION public.store_staff_legacy_read_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    RAISE EXCEPTION 'legacy staff associations are read only' USING ERRCODE='23514';
END
$$;

CREATE FUNCTION public.stores_identity_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF ROW(NEW.id,NEW.tenant_id,NEW.organization_id,NEW.owner_user_id,NEW.created_at)
       IS DISTINCT FROM ROW(OLD.id,OLD.tenant_id,OLD.organization_id,OLD.owner_user_id,OLD.created_at)
       OR NEW.version <> OLD.version + 1 THEN
        RAISE EXCEPTION 'store identity or version is immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE FUNCTION public.stores_platform_name_immutable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.name IS DISTINCT FROM OLD.name THEN
        RAISE EXCEPTION 'store name is owned by the platform' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE FUNCTION public.stores_schema_fingerprint() RETURNS text
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
SELECT md5(COALESCE(string_agg(item,E'\n' ORDER BY item),'')) FROM (
 SELECT 'relation|'||c.relname||'|'||c.relkind::text||'|'||c.relpersistence::text||'|'||c.relrowsecurity AS item FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','v','m','S','p') AND c.relname<>'_sqlx_migrations'
 UNION ALL SELECT 'column|'||c.relname||'|'||a.attname||'|'||format_type(a.atttypid,a.atttypmod)||'|'||a.attnotnull||'|'||COALESCE(pg_get_expr(d.adbin,d.adrelid),'') FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE n.nspname='public' AND a.attnum>0 AND NOT a.attisdropped AND c.relkind IN ('r','p') AND c.relname<>'_sqlx_migrations'
 UNION ALL SELECT 'constraint|'||c.relname||'|'||k.conname||'|'||pg_get_constraintdef(k.oid,true) FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname<>'_sqlx_migrations'
 UNION ALL SELECT 'index|'||tablename||'|'||indexname||'|'||indexdef FROM pg_indexes WHERE schemaname='public' AND tablename<>'_sqlx_migrations'
 UNION ALL SELECT 'trigger|'||c.relname||'|'||t.tgname||'|'||t.tgenabled::text||'|'||pg_get_triggerdef(t.oid,true) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND NOT t.tgisinternal
 UNION ALL SELECT 'function|'||p.proname||'|'||pg_get_function_identity_arguments(p.oid)||'|'||pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'
) schema_items
$$;

SET default_table_access_method = heap;

CREATE TABLE public._stores_schema_contract (
    singleton boolean DEFAULT true NOT NULL,
    fingerprint text NOT NULL,
    CONSTRAINT _stores_schema_contract_fingerprint_check CHECK ((length(fingerprint) = 32)),
    CONSTRAINT _stores_schema_contract_singleton_check CHECK (singleton)
);

CREATE TABLE public.store_audit_events (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    actor_user_id uuid NOT NULL,
    resource_code character varying(32) NOT NULL,
    action_code character varying(16) NOT NULL,
    entity_id uuid NOT NULL,
    request_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT store_audit_events_action_code_check CHECK (((action_code)::text = ANY ((ARRAY['create'::character varying, 'update'::character varying, 'assign'::character varying, 'unassign'::character varying])::text[]))),
    CONSTRAINT store_audit_events_actor_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(actor_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(actor_user_id), 8) & 192) = 128))),
    CONSTRAINT store_audit_events_entity_id_uuid_v7 CHECK ((((get_byte(uuid_send(entity_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(entity_id), 8) & 192) = 128))),
    CONSTRAINT store_audit_events_id_uuid_v7 CHECK ((((get_byte(uuid_send(id), 6) >> 4) = 7) AND ((get_byte(uuid_send(id), 8) & 192) = 128))),
    CONSTRAINT store_audit_events_organization_id_uuid_v7 CHECK ((((get_byte(uuid_send(organization_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(organization_id), 8) & 192) = 128))),
    CONSTRAINT store_audit_events_request_id_uuid_v7 CHECK ((((get_byte(uuid_send(request_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(request_id), 8) & 192) = 128))),
    CONSTRAINT store_audit_events_resource_code_check CHECK (((resource_code)::text = ANY ((ARRAY['store'::character varying, 'store-staff'::character varying, 'store-daily-report'::character varying])::text[]))),
    CONSTRAINT store_audit_events_tenant_id_uuid_v7 CHECK ((((get_byte(uuid_send(tenant_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(tenant_id), 8) & 192) = 128)))
);

CREATE TABLE public.store_daily_reports (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    store_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    business_date date NOT NULL,
    amount numeric(18,2) NOT NULL,
    note text DEFAULT ''::text NOT NULL,
    version integer DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT store_daily_reports_amount_check CHECK (((amount >= (0)::numeric) AND (amount <> 'NaN'::numeric))),
    CONSTRAINT store_daily_reports_business_date_check CHECK (((business_date >= '0001-01-01'::date) AND (business_date <= '9999-12-31'::date))),
    CONSTRAINT store_daily_reports_id_uuid_v7 CHECK ((((get_byte(uuid_send(id), 6) >> 4) = 7) AND ((get_byte(uuid_send(id), 8) & 192) = 128))),
    CONSTRAINT store_daily_reports_note_check CHECK ((length(note) <= 8192)),
    CONSTRAINT store_daily_reports_organization_id_uuid_v7 CHECK ((((get_byte(uuid_send(organization_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(organization_id), 8) & 192) = 128))),
    CONSTRAINT store_daily_reports_owner_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(owner_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(owner_user_id), 8) & 192) = 128))),
    CONSTRAINT store_daily_reports_store_id_uuid_v7 CHECK ((((get_byte(uuid_send(store_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(store_id), 8) & 192) = 128))),
    CONSTRAINT store_daily_reports_tenant_id_uuid_v7 CHECK ((((get_byte(uuid_send(tenant_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(tenant_id), 8) & 192) = 128))),
    CONSTRAINT store_daily_reports_version_check CHECK ((version > 0))
);

CREATE TABLE public.store_staff (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    store_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    user_id uuid NOT NULL,
    display_name character varying(100) NOT NULL,
    created_by_user_id uuid NOT NULL,
    version integer DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    removed_at timestamp with time zone,
    CONSTRAINT store_staff_check CHECK ((owner_user_id = user_id)),
    CONSTRAINT store_staff_created_by_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(created_by_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(created_by_user_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_display_name_check CHECK ((length(TRIM(BOTH FROM display_name)) > 0)),
    CONSTRAINT store_staff_id_uuid_v7 CHECK ((((get_byte(uuid_send(id), 6) >> 4) = 7) AND ((get_byte(uuid_send(id), 8) & 192) = 128))),
    CONSTRAINT store_staff_organization_id_uuid_v7 CHECK ((((get_byte(uuid_send(organization_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(organization_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_owner_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(owner_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(owner_user_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_store_id_uuid_v7 CHECK ((((get_byte(uuid_send(store_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(store_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_tenant_id_uuid_v7 CHECK ((((get_byte(uuid_send(tenant_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(tenant_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(user_id), 8) & 192) = 128))),
    CONSTRAINT store_staff_version_check CHECK ((version > 0))
);

CREATE TABLE public.stores (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    name character varying(200) NOT NULL,
    business_status character varying(6) NOT NULL,
    business_contact character varying(200) NOT NULL,
    manager_user_id uuid,
    manager_display_name character varying(100),
    version integer DEFAULT 1 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT stores_business_status_check CHECK (((business_status)::text = ANY ((ARRAY['OPEN'::character varying, 'CLOSED'::character varying])::text[]))),
    CONSTRAINT stores_check CHECK (((manager_user_id IS NULL) = (manager_display_name IS NULL))),
    CONSTRAINT stores_id_uuid_v7 CHECK ((((get_byte(uuid_send(id), 6) >> 4) = 7) AND ((get_byte(uuid_send(id), 8) & 192) = 128))),
    CONSTRAINT stores_manager_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(manager_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(manager_user_id), 8) & 192) = 128))),
    CONSTRAINT stores_name_check CHECK ((length(TRIM(BOTH FROM name)) > 0)),
    CONSTRAINT stores_organization_id_uuid_v7 CHECK ((((get_byte(uuid_send(organization_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(organization_id), 8) & 192) = 128))),
    CONSTRAINT stores_owner_user_id_uuid_v7 CHECK ((((get_byte(uuid_send(owner_user_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(owner_user_id), 8) & 192) = 128))),
    CONSTRAINT stores_tenant_id_uuid_v7 CHECK ((((get_byte(uuid_send(tenant_id), 6) >> 4) = 7) AND ((get_byte(uuid_send(tenant_id), 8) & 192) = 128))),
    CONSTRAINT stores_version_check CHECK ((version > 0))
);

ALTER TABLE ONLY public._stores_schema_contract
    ADD CONSTRAINT _stores_schema_contract_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.store_audit_events
    ADD CONSTRAINT store_audit_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.store_daily_reports
    ADD CONSTRAINT store_daily_reports_day_key UNIQUE (tenant_id, store_id, business_date);

ALTER TABLE ONLY public.store_daily_reports
    ADD CONSTRAINT store_daily_reports_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.store_staff
    ADD CONSTRAINT store_staff_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.stores
    ADD CONSTRAINT stores_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.stores
    ADD CONSTRAINT stores_tenant_id_id_organization_id_key UNIQUE (tenant_id, id, organization_id);

ALTER TABLE ONLY public.stores
    ADD CONSTRAINT stores_tenant_id_organization_id_key UNIQUE (tenant_id, organization_id);

CREATE INDEX store_audit_events_scope ON public.store_audit_events USING btree (tenant_id, organization_id, created_at, id);

CREATE INDEX store_daily_reports_scope ON public.store_daily_reports USING btree (tenant_id, organization_id, owner_user_id, business_date, store_id, id);

CREATE UNIQUE INDEX store_staff_active_member_key ON public.store_staff USING btree (tenant_id, store_id, user_id) WHERE (removed_at IS NULL);

CREATE INDEX store_staff_scope ON public.store_staff USING btree (tenant_id, organization_id, owner_user_id, store_id, id) WHERE (removed_at IS NULL);

CREATE INDEX stores_tenant_owner ON public.stores USING btree (tenant_id, owner_user_id, organization_id, id);

CREATE TRIGGER store_audit_append_only BEFORE DELETE OR UPDATE ON public.store_audit_events FOR EACH ROW EXECUTE FUNCTION public.store_audit_append_only();

CREATE TRIGGER store_daily_report_identity_immutable BEFORE UPDATE ON public.store_daily_reports FOR EACH ROW EXECUTE FUNCTION public.store_daily_report_identity_immutable();

CREATE TRIGGER store_staff_identity_immutable BEFORE UPDATE ON public.store_staff FOR EACH ROW EXECUTE FUNCTION public.store_staff_identity_immutable();

CREATE TRIGGER store_staff_legacy_read_only BEFORE INSERT OR DELETE OR UPDATE OR TRUNCATE ON public.store_staff FOR EACH STATEMENT EXECUTE FUNCTION public.store_staff_legacy_read_only();

CREATE TRIGGER stores_identity_immutable BEFORE UPDATE ON public.stores FOR EACH ROW EXECUTE FUNCTION public.stores_identity_immutable();

CREATE TRIGGER stores_platform_name_immutable BEFORE UPDATE OF name ON public.stores FOR EACH ROW EXECUTE FUNCTION public.stores_platform_name_immutable();

ALTER TABLE ONLY public.store_daily_reports
    ADD CONSTRAINT store_daily_reports_tenant_id_store_id_organization_id_fkey FOREIGN KEY (tenant_id, store_id, organization_id) REFERENCES public.stores(tenant_id, id, organization_id);

ALTER TABLE ONLY public.store_staff
    ADD CONSTRAINT store_staff_tenant_id_store_id_organization_id_fkey FOREIGN KEY (tenant_id, store_id, organization_id) REFERENCES public.stores(tenant_id, id, organization_id);

INSERT INTO public._stores_schema_contract(singleton,fingerprint) VALUES(true,public.stores_schema_fingerprint());
SET check_function_bodies = true;
SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false);

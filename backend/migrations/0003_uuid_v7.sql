-- UUID-U02 is an identity epoch boundary. This migration is for an empty
-- target, or a previously verified catalog whose identities are already v7.
-- Existing v4 business rows deliberately fail validation and roll back; no
-- source identity, history row, migration record or old baseline is rewritten.
LOCK TABLE public.stores, public.store_staff, public.store_daily_reports,
    public.store_audit_events IN SHARE ROW EXCLUSIVE MODE;
DO $$
BEGIN
    IF NOT (SELECT count(*)=1 AND COALESCE(bool_and(
        v2.fingerprint=public.stores_schema_fingerprint_v2()
        AND v2.legacy_fingerprint=v1.fingerprint),false)
        FROM public._stores_schema_contract_v2 AS v2
        JOIN public._stores_schema_contract AS v1 USING(singleton)
        WHERE v2.singleton) THEN
        RAISE EXCEPTION 'v2 business schema contract mismatch' USING ERRCODE='23514';
    END IF;
END
$$;

-- uuid_send and byte checks are available on PostgreSQL 17. Application code
-- supplies every UUID; no database generator or PG18 uuidv7() is required.
ALTER TABLE public.stores
    ADD CONSTRAINT stores_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(id),8) & 192)=128),
    ADD CONSTRAINT stores_tenant_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),8) & 192)=128),
    ADD CONSTRAINT stores_organization_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),8) & 192)=128),
    ADD CONSTRAINT stores_owner_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),8) & 192)=128),
    ADD CONSTRAINT stores_manager_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(manager_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(manager_user_id),8) & 192)=128);

ALTER TABLE public.store_staff
    ADD CONSTRAINT store_staff_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(id),8) & 192)=128),
    ADD CONSTRAINT store_staff_tenant_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),8) & 192)=128),
    ADD CONSTRAINT store_staff_store_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(store_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(store_id),8) & 192)=128),
    ADD CONSTRAINT store_staff_organization_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),8) & 192)=128),
    ADD CONSTRAINT store_staff_owner_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),8) & 192)=128),
    ADD CONSTRAINT store_staff_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(user_id),8) & 192)=128),
    ADD CONSTRAINT store_staff_created_by_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(created_by_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(created_by_user_id),8) & 192)=128);

ALTER TABLE public.store_daily_reports
    ADD CONSTRAINT store_daily_reports_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(id),8) & 192)=128),
    ADD CONSTRAINT store_daily_reports_tenant_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),8) & 192)=128),
    ADD CONSTRAINT store_daily_reports_store_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(store_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(store_id),8) & 192)=128),
    ADD CONSTRAINT store_daily_reports_organization_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),8) & 192)=128),
    ADD CONSTRAINT store_daily_reports_owner_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(owner_user_id),8) & 192)=128);

ALTER TABLE public.store_audit_events
    ADD CONSTRAINT store_audit_events_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(id),8) & 192)=128),
    ADD CONSTRAINT store_audit_events_tenant_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(tenant_id),8) & 192)=128),
    ADD CONSTRAINT store_audit_events_organization_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(organization_id),8) & 192)=128),
    ADD CONSTRAINT store_audit_events_actor_user_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(actor_user_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(actor_user_id),8) & 192)=128),
    ADD CONSTRAINT store_audit_events_entity_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(entity_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(entity_id),8) & 192)=128),
    ADD CONSTRAINT store_audit_events_request_id_uuid_v7 CHECK ((pg_catalog.get_byte(pg_catalog.uuid_send(request_id),6) >> 4)=7 AND (pg_catalog.get_byte(pg_catalog.uuid_send(request_id),8) & 192)=128);

-- Preserve both prior snapshots. Only this verified transition may establish
-- the new owner contract; readiness never refreshes a stored fingerprint.
CREATE TABLE public._stores_schema_contract_v3 (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    legacy_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL
);
CREATE FUNCTION public.stores_schema_fingerprint_v3() RETURNS TEXT LANGUAGE sql STABLE AS $$
    SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(item,E'\n' ORDER BY item),'')) FROM (
        SELECT 'v2|'||public.stores_schema_fingerprint_v2() AS item
        UNION ALL
        SELECT 'column|'||c.relname||'|'||a.attnum||'|'||a.attname||'|'||pg_catalog.format_type(a.atttypid,a.atttypmod)||'|'||a.attnotnull||'|'||COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),'')
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
        LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
        WHERE n.nspname='public' AND c.relname='_stores_schema_contract_v3'
        UNION ALL
        SELECT 'constraint|'||c.relname||'|'||k.conname||'|'||pg_catalog.pg_get_constraintdef(k.oid,true)
        FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND c.relname='_stores_schema_contract_v3'
        UNION ALL
        SELECT 'index|'||tablename||'|'||indexname||'|'||indexdef FROM pg_catalog.pg_indexes
        WHERE schemaname='public' AND tablename='_stores_schema_contract_v3'
        UNION ALL
        SELECT 'trigger|'||c.relname||'|'||t.tgname||'|'||t.tgenabled::text||'|'||pg_catalog.pg_get_triggerdef(t.oid,true)
        FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND NOT t.tgisinternal AND c.relname='_stores_schema_contract_v3'
        UNION ALL
        SELECT 'function|'||p.proname||'|'||pg_catalog.pg_get_functiondef(p.oid)
        FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
        WHERE n.nspname='public' AND p.proname IN ('stores_schema_fingerprint_v3')
    ) contracts
$$;
INSERT INTO public._stores_schema_contract_v3(singleton,legacy_fingerprint,fingerprint)
    SELECT true,fingerprint,public.stores_schema_fingerprint_v3()
    FROM public._stores_schema_contract_v2 WHERE singleton;

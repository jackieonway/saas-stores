-- Preserve the v1 migration and its original schema baseline. Refuse to build
-- a new baseline on top of a database whose v1 catalog has already drifted.
LOCK TABLE public.stores, public.store_staff IN SHARE ROW EXCLUSIVE MODE;
DO $$
BEGIN
    IF NOT (SELECT count(*)=1 AND COALESCE(bool_and(fingerprint=public.stores_schema_fingerprint()),false)
            FROM public._stores_schema_contract WHERE singleton) THEN
        RAISE EXCEPTION 'legacy business schema contract mismatch' USING ERRCODE='23514';
    END IF;
END
$$;

-- The existing name is a historical cache. Runtime responses use the platform
-- STORE organization name; neither an old binary nor a new profile update may
-- rename this cache. An unchanged name remains compatible with older SQL.
CREATE FUNCTION public.stores_platform_name_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.name IS DISTINCT FROM OLD.name THEN
        RAISE EXCEPTION 'store name is owned by the platform' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER stores_platform_name_immutable BEFORE UPDATE OF name ON public.stores
    FOR EACH ROW EXECUTE FUNCTION public.stores_platform_name_immutable();

-- Existing staff associations remain intact as history. A statement guard also
-- blocks empty writes and TRUNCATE from an old running service; SELECT is allowed.
CREATE FUNCTION public.store_staff_legacy_read_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'legacy staff associations are read only' USING ERRCODE='23514';
END
$$;
CREATE TRIGGER store_staff_legacy_read_only BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.store_staff
    FOR EACH STATEMENT EXECUTE FUNCTION public.store_staff_legacy_read_only();

-- V1's fingerprint includes the complete public inventory, so new objects make
-- that historical snapshot differ by design. Keep its function and stored value
-- unchanged, and record a separate v2 contract after the verified transition.
CREATE TABLE public._stores_schema_contract_v2 (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    legacy_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL
);
CREATE FUNCTION public.stores_schema_fingerprint_v2() RETURNS TEXT LANGUAGE sql STABLE AS $$
    SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(item,E'\n' ORDER BY item),'')) FROM (
        SELECT 'v1|'||public.stores_schema_fingerprint() AS item
        UNION ALL
        SELECT 'column|'||c.relname||'|'||a.attnum||'|'||a.attname||'|'||pg_catalog.format_type(a.atttypid,a.atttypmod)||'|'||a.attnotnull||'|'||COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),'')
        FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
        LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum
        WHERE n.nspname='public' AND c.relname='_stores_schema_contract_v2'
        UNION ALL
        SELECT 'constraint|'||c.relname||'|'||k.conname||'|'||pg_catalog.pg_get_constraintdef(k.oid,true)
        FROM pg_catalog.pg_constraint k JOIN pg_catalog.pg_class c ON c.oid=k.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND c.relname='_stores_schema_contract_v2'
        UNION ALL
        SELECT 'index|'||tablename||'|'||indexname||'|'||indexdef FROM pg_catalog.pg_indexes
        WHERE schemaname='public' AND tablename='_stores_schema_contract_v2'
        UNION ALL
        SELECT 'trigger|'||c.relname||'|'||t.tgname||'|'||t.tgenabled::text||'|'||pg_catalog.pg_get_triggerdef(t.oid,true)
        FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND NOT t.tgisinternal AND c.relname='_stores_schema_contract_v2'
        UNION ALL
        SELECT 'function|'||p.proname||'|'||pg_catalog.pg_get_functiondef(p.oid)
        FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
        WHERE n.nspname='public' AND p.proname IN ('stores_platform_name_immutable','store_staff_legacy_read_only','stores_schema_fingerprint_v2')
    ) contracts
$$;
INSERT INTO public._stores_schema_contract_v2(singleton,legacy_fingerprint,fingerprint)
    SELECT true,fingerprint,public.stores_schema_fingerprint_v2() FROM public._stores_schema_contract WHERE singleton;

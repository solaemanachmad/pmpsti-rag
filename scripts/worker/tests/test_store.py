from worker import store


def test_production_guard_matches_pooled_and_direct_neon_hosts():
    prod = "postgresql://x:y@ep-calm-dawn-b4n290kt.c-6.us-east-2.aws.neon.tech/neondb?sslmode=require"
    assert store.is_production("postgresql://u:p@ep-calm-dawn-b4n290kt-pooler.c-6.us-east-2.aws.neon.tech/neondb", prod)
    assert store.is_production(prod, prod)
    assert not store.is_production("postgresql://u:p@ep-other-123.c-6.us-east-2.aws.neon.tech/neondb", prod)
    assert not store.is_production("postgresql://u:p@ep-other-123.c-6.us-east-2.aws.neon.tech/neondb", "")

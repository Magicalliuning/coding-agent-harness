pub const DATABASE_URL_ENV: &str = "HARNESS_DATABASE_URL";

pub const MIGRATIONS_DIR: &str = "migrations";

#[must_use]
pub fn baseline_summary() -> String {
    format!(
        "{} uses EventLog schema v{} with migrations in {MIGRATIONS_DIR}",
        harness_core::PRODUCT_NAME,
        harness_events::eventlog_schema_version()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_names_database_env_and_migration_path() {
        assert_eq!(DATABASE_URL_ENV, "HARNESS_DATABASE_URL");
        assert_eq!(MIGRATIONS_DIR, "migrations");
        assert!(baseline_summary().contains("EventLog schema v1"));
    }
}

use proptest::test_runner::{Config, FileFailurePersistence};

pub fn proptest_config() -> Config {
    Config {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..Config::default()
    }
}

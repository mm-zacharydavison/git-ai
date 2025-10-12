// Integration test file to make the repos module compile and be testable
mod repos;

// Re-export for convenience
pub use repos::test_file::{AuthorType, ExpectedLine, ExpectedLineExt, TestFile};
pub use repos::test_repo::TestRepo;

#[test]
fn test_repos_module_loads() {
    // This test just ensures the repos module compiles successfully
    assert!(true);
}

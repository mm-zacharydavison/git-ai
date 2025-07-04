use git_ai::tmp_repo::TmpRepo;
use tempfile::tempdir;

#[test]
fn test_simple_additions() {
    // Create a temporary directory
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    // Create a new TmpRepo
    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    // Test writing a file
    tmp_repo
        .write_file("test.txt", "Hello, World!", true)
        .unwrap();

    // Test triggering a checkpoint
    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    // Test writing a file
    tmp_repo
        .write_file("test.txt", "Hello, World!\nGoodbye Now!", true)
        .unwrap();

    tmp_repo.trigger_checkpoint_with_author("Claude").unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    // Verify the repository path
    // assert_eq!(tmp_repo.path(), &repo_path);
}

use git_ai::tmp_repo::TmpRepo;
use insta::assert_debug_snapshot;
use tempfile::tempdir;

#[test]
fn test_simple_additions() {
    // Create a temporary directory
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let tmp_repo = TmpRepo::new(repo_path.clone()).unwrap();

    let mut file = tmp_repo.write_file("test.txt", "Line1\n", true).unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    file.append("Line 2\nLine 3\n").unwrap();

    tmp_repo.trigger_checkpoint_with_author("Claude").unwrap();

    tmp_repo.commit_with_message("Initial commit").unwrap();

    let blame = tmp_repo.blame_for_file(&file, None).unwrap();
    assert_debug_snapshot!(blame);
}

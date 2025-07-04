use git_ai::tmp_repo::TmpRepo;
use tempfile::tempdir;

#[test]
fn test_tmp_file_operations() {
    let tmp_dir = tempdir().unwrap().path().to_path_buf();
    let repo = TmpRepo::new(tmp_dir).unwrap();

    // Create a file and get a TmpFile handle
    let mut hello_world_ts = repo
        .write_file("src/hello.ts", "Line 1\nLine 2\nLine 3\n", true)
        .unwrap();

    repo.trigger_checkpoint_with_author("Claude").unwrap();

    // Append content
    hello_world_ts.append("Line 4\nLine 5\nLine 6").unwrap();

    repo.trigger_checkpoint_with_author("Testman").unwrap();

    repo.commit_with_message("test-commit").unwrap();

    let blame = repo.blame_for_file(&hello_world_ts, None).unwrap();
    println!("Blame: {:?}", blame);
}

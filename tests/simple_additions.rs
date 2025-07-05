use git_ai::tmp_repo::TmpRepo;
use insta::assert_debug_snapshot;
use tempfile::tempdir;

#[test]
fn test_simple_additions_empty_repo() {
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

#[test]
fn test_simple_additions_with_base_commit() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

    lines
        .append("NEW LINEs From Claude!\nHello\nWorld\n")
        .unwrap();

    tmp_repo.trigger_checkpoint_with_author("Claude").unwrap();

    tmp_repo.commit_with_message("next commit").unwrap();

    let blame = tmp_repo.blame_for_file(&lines, None).unwrap();

    assert_debug_snapshot!(blame);
}

#[test]
fn test_simple_additions_on_top_of_ai_contributions() {
    let tmp_dir = tempdir().unwrap();
    let repo_path = tmp_dir.path().to_path_buf();

    let (tmp_repo, mut lines, _) = TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

    lines
        .append("NEW LINEs From Claude!\nHello\nWorld\n")
        .unwrap();

    tmp_repo.trigger_checkpoint_with_author("Claude").unwrap();

    tmp_repo.commit_with_message("next commit 1").unwrap();

    lines.replace_range(34, 35, "HUMAN ON AI\n").unwrap();

    tmp_repo
        .trigger_checkpoint_with_author("test_user")
        .unwrap();

    tmp_repo.commit_with_message("next commit 2").unwrap();

    let blame = tmp_repo.blame_for_file(&lines, Some((30, 35))).unwrap();
    assert_debug_snapshot!(blame);
}

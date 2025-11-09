mod repos;
use git_ai::authorship::stats::CommitStats;
use repos::test_file::ExpectedLineExt;
use repos::test_repo::TestRepo;
use serde_json;

#[test]
fn test_authorship_log_stats() {
    let repo = TestRepo::new();

    // Create an initial commit
    let mut readme = repo.filename("README.md");
    readme.set_contents(lines!["# Project"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // AI creates a brand new file with planets
    let mut file = repo.filename("planets.txt");
    file.set_contents(lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune".ai(),
        "Pluto (dwarf)".ai(),
    ]);

    file.set_contents(lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune (override)".human(),
        "Pluto (dwarf)".ai(),
    ]);

    // First commit should have all the planets
    let first_commit = repo.stage_all_and_commit("Add planets").unwrap();

    file.assert_lines_and_blame(lines![
        "Mercury".human(),
        "Venus".human(),
        "Earth".ai(),
        "Mars".ai(),
        "Jupiter".human(),
        "Saturn".ai(),
        "Uranus".ai(),
        "Neptune (override)".human(),
        "Pluto (dwarf)".ai(),
    ]);

    assert_eq!(first_commit.authorship_log.attestations.len(), 1);

    let mut stats = repo.git_ai(&["stats", "--json"]).unwrap();
    stats = stats.split("}}}").next().unwrap().to_string() + "}}}";
    let stats: CommitStats = serde_json::from_str(&stats).unwrap();
    assert_eq!(stats.human_additions, 4);
    assert_eq!(stats.mixed_additions, 1);
    assert_eq!(stats.ai_additions, 6); // Includes the one mixed line (Neptune (override))
    assert_eq!(stats.ai_accepted, 5);
    assert_eq!(stats.total_ai_additions, 11);
    assert_eq!(stats.total_ai_deletions, 11);
    assert_eq!(stats.git_diff_deleted_lines, 0);
    assert_eq!(stats.git_diff_added_lines, 9);

    assert_eq!(stats.tool_model_breakdown.len(), 1);
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .ai_additions,
        6
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .ai_accepted,
        5
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .total_ai_additions,
        11
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .total_ai_deletions,
        11
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .mixed_additions,
        1
    );
    assert_eq!(
        stats
            .tool_model_breakdown
            .get("mock_ai::unknown")
            .unwrap()
            .time_waiting_for_ai,
        0
    );
}

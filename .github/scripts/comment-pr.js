const { owner, repo, number } = context.issue;
const commitSha = process.env.COMMIT_SHA;
const statsOutput = process.env.STATS_OUTPUT;
const eventName = process.env.EVENT_NAME;

let title, body;

if (eventName === 'push') {
  title = '🚀 **Code Pushed!**';
  body = statsOutput;
} else {
  title = '📝 **PR Commit Analysis**';
  body = statsOutput;
}

const comment = title + '\n\n' + body;

await github.rest.issues.createComment({
  owner,
  repo,
  issue_number: number,
  body: comment
}); 
const { owner, repo, number } = context.issue;
const commitSha = process.env.COMMIT_SHA;
const statsOutput = process.env.STATS_OUTPUT;
const eventName = process.env.EVENT_NAME;

let body;
let title = 'Live demo of git-ai:';

if (eventName === 'push') {
  body = statsOutput;
} else {
  body = statsOutput;
}

const comment = title + '\n\n' + body;

await github.rest.issues.createComment({
  owner,
  repo,
  issue_number: number,
  body: comment
}); 
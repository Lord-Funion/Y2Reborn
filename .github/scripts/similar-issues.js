// Finds earlier issues that look like a new one, for a hint comment. No dependencies: the
// workflow passes GitHub's client in, and the scoring below is plain and tested by
// tests/test_similar_issues.py.

// Words that say nothing about the subject of an issue.
const STOP = new Set(('a an and are as at be but by can could do does for from has have how i if in into is it its '
  + 'me my no not of on or please possible request feature option ability able add adding allow make '
  + 'should so some than that the their them then there this to up use using want was when where which '
  + 'while will with would you your y2 y2linux y2reborn reborn player device bug issue problem '
  + 'show showing view screen top any more give get getting new other certain instead beta idea').split(' '));

/** The words of a title that carry its subject: lower case, no duplicates, stop words left out. */
function keywords(title) {
  const words = (title || '').toLowerCase().replace(/[^a-z0-9%+ ]+/g, ' ').split(/\s+/)
    .map((word) => (word.length > 4 && word.endsWith('s') ? word.slice(0, -1) : word))
    .filter((word) => word.length >= 3 && !STOP.has(word));
  return [...new Set(words)];
}

/** How much two titles share: the share of the shorter one's words found in the other. */
function likeness(a, b) {
  const [left, right] = [keywords(a), keywords(b)];
  const shared = left.filter((word) => right.includes(word)).length;
  const shorter = Math.min(left.length, right.length);
  return { shared, share: shorter ? shared / shorter : 0 };
}

/** The issues worth showing for `title`: three shared words that are at least half of the shorter
 * title, or two that are two thirds of it; best first, at most `limit`. Tried against the 150
 * newest issues of each tracker: a looser rule commented on every second issue with ones that
 * were only about the same part of the player. */
function similar(title, candidates, limit = 3) {
  return candidates
    .map((issue) => ({ issue, ...likeness(title, issue.title) }))
    .filter((match) => (match.shared >= 3 && match.share >= 0.5) || (match.shared >= 2 && match.share >= 0.66))
    .sort((x, y) => y.share - x.share || y.shared - x.shared)
    .slice(0, limit)
    .map((match) => match.issue);
}

/** The comment for the matches found. */
function comment(matches) {
  const lines = matches.map((issue) => {
    const state = issue.state === 'open' ? 'open' : 'closed';
    return `- [${issue.title}](${issue.html_url}) (${state})`;
  });
  return [
    'Thanks for writing in! These earlier issues look similar. This is an automatic hint, so it may be wrong:',
    '',
    ...lines,
    '',
    'If one of them is the same thing, please add your comment there and close this one. '
      + "Ideas that were asked before and won't be built are listed in "
      + '[this page](https://github.com/SchulzCode/Y2Linux/blob/main/docs/NOT-PLANNED.md).',
  ].join('\n');
}

/** Run for a newly opened issue: search both repositories by its words and comment when something fits. */
async function run({ github, context }) {
  const issue = context.payload.issue;
  const words = keywords(issue.title);
  if (words.length < 2) return;
  const found = new Map();
  for (const repo of ['SchulzCode/Y2Linux', 'SchulzCode/Y2Reborn']) {
    // Any of the words, in titles; the scoring decides what fits.
    const q = `repo:${repo} is:issue in:title ${words.slice(0, 6).join(' OR ')}`;
    const result = await github.rest.search.issuesAndPullRequests({ q, per_page: 30 });
    for (const item of result.data.items) {
      if (item.html_url !== issue.html_url && !item.pull_request) found.set(item.html_url, item);
    }
  }
  const matches = similar(issue.title, [...found.values()]);
  if (!matches.length) return;
  await github.rest.issues.createComment({
    owner: context.repo.owner, repo: context.repo.repo, issue_number: issue.number, body: comment(matches),
  });
}

module.exports = { keywords, likeness, similar, comment, run };

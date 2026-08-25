// Posts a release's notes into Discord's #announcements.
//
// The notes are already written once, in web/src/content/releases/<version>.md,
// which is what the website serves. This reads that same file and posts it to a
// Discord webhook, so the announcement and the release page cannot say different
// things.
//
//   node scripts/discord-release.mjs 0.2.0 --dry-run   print it, send nothing
//   node scripts/discord-release.mjs 0.2.0             post it
//   node scripts/discord-release.mjs 0.2.0 --no-ping   post it quietly
//
// A release pings @everyone, as its own short message ahead of the notes. That
// needs MENTION_EVERYONE granted to @everyone in #announcements, set once
// by hand in the channel's permission settings.
//
// The webhook address comes from DISCORD_RELEASE_WEBHOOK. It is a secret in its
// own right — anyone holding it can post into the channel — so it is never
// printed, not even on failure.
//
// Called by the `announce` job in .github/workflows/release.yml, which runs it
// only for a plain vX.Y.Z tag. Pre-release tags (v0.2.0-rc1) are not announced.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const NOTES_DIR = join(ROOT, 'web', 'src', 'content', 'releases');
const SITE = 'https://lumitlab.com';
const REPO = 'https://github.com/luminalmvm/Lumit';
const LIMIT = 1900; // Discord's cap is 2000; leave room for a stray wide glyph

const args = process.argv.slice(2);
const DRY = args.includes('--dry-run');
const PING = !args.includes('--no-ping');
const version = args.find((a) => !a.startsWith('--'))?.replace(/^v/, '');

const die = (message) => {
  console.error(`\n  ${message}\n`);
  process.exit(1);
};

/** Split `---` frontmatter off the top. Values are plain scalars, so no YAML. */
function splitFrontmatter(raw) {
  const m = raw.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!m) return { meta: {}, body: raw };
  const meta = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = line.match(/^([A-Za-z][A-Za-z0-9_]*):\s*(.*)$/);
    if (kv) meta[kv[1]] = kv[2].trim().replace(/^["']|["']$/g, '');
  }
  return { meta, body: raw.slice(m[0].length) };
}

/**
 * Join the hard wrapping back up.
 *
 * The source is wrapped at about 80 columns for the sake of the repository, but
 * Discord honours every single newline, so a wrapped paragraph arrives as a
 * column of ragged fragments. Headings, list items, quotes and fenced code keep
 * their own lines; anything else is glued to the line above it.
 */
function unwrap(body) {
  const out = [];
  let buffer = '';
  let fenced = false;

  const flush = () => {
    if (buffer) out.push(buffer);
    buffer = '';
  };

  for (const line of body.split(/\r?\n/)) {
    if (/^\s*```/.test(line)) {
      flush();
      out.push(line);
      fenced = !fenced;
      continue;
    }
    if (fenced) {
      out.push(line);
      continue;
    }
    if (!line.trim()) {
      flush();
      out.push('');
      continue;
    }
    // A line that starts its own block: heading, bullet, number, quote, table,
    // or a horizontal rule.
    if (/^\s{0,3}(#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?|\||---\s*$)/.test(line)) {
      flush();
      buffer = line.replace(/\s+$/, '');
      continue;
    }
    buffer = buffer ? `${buffer} ${line.trim()}` : line.replace(/\s+$/, '');
  }
  flush();

  // Collapse the runs of blank lines the flushing can leave behind.
  return out
    .filter((l, i) => l.trim() || out[i - 1]?.trim())
    .join('\n')
    .trim();
}

/** Break a document into Discord-sized messages, preferring section breaks. */
function chunk(text) {
  const messages = [];
  let current = []; // the blocks of the message being filled

  const size = () => current.reduce((n, b) => n + b.length + 2, -2);
  const isHeading = (block) => /^\s{0,3}#{1,6}\s/.test(block);

  const push = () => {
    if (current.length) messages.push(current.join('\n\n'));
    current = [];
  };

  /**
   * End the message here, but carry any trailing headings over: a message that
   * ends on "### Language framework" strands the heading from the list it
   * introduces, and the next message opens on bullets about nothing.
   */
  const breakHere = () => {
    const carried = [];
    while (current.length > 1 && isHeading(current[current.length - 1])) {
      carried.unshift(current.pop());
    }
    push();
    current = carried;
  };

  for (const block of text.split(/\n\n+/).filter((b) => b.trim())) {
    // A single block over the limit has no section break to use, so it is cut
    // on its own lines as a last resort.
    if (block.length > LIMIT) {
      breakHere();
      push();
      let lines = '';
      for (const line of block.split('\n')) {
        if (lines.length + line.length + 1 > LIMIT) {
          messages.push(lines);
          lines = '';
        }
        lines += (lines ? '\n' : '') + line;
      }
      current = lines ? [lines] : [];
      continue;
    }
    if (current.length && size() + block.length + 2 > LIMIT) breakHere();
    current.push(block);
  }
  push();
  return messages;
}

async function post(url, content, mentionEveryone = false) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      content,
      // Only the lone @everyone message is allowed to mention anybody. The
      // notes that follow carry no permission at all, so a stray "@everyone"
      // written in a release note cannot ping the server a second time.
      allowed_mentions: { parse: mentionEveryone ? ['everyone'] : [] },
    }),
  });
  if (res.status === 429) {
    const wait = Number(res.headers.get('retry-after') ?? 1) * 1000 + 250;
    await new Promise((r) => setTimeout(r, wait));
    return post(url, content, mentionEveryone);
  }
  if (!res.ok) {
    // The body can echo the address back, so only the status is shown.
    throw new Error(`Discord refused the message (HTTP ${res.status}).`);
  }
}

async function main() {
  if (!version) {
    die('Which version? For example: node scripts/discord-release.mjs 0.2.0');
  }

  const path = join(NOTES_DIR, `${version}.md`);
  let raw;
  try {
    raw = readFileSync(path, 'utf8');
  } catch {
    die(
      `No release notes at web/src/content/releases/${version}.md.\n` +
        '  Write them there — the website serves the same file — and run this again.',
    );
  }

  const { meta, body } = splitFrontmatter(raw);
  const title = meta.title || `Lumit v${version}`;

  const document = [
    `# ${title}`,
    '',
    ...(meta.description ? [meta.description, ''] : []),
    `Full notes: <${SITE}/releases/${version}>`,
    `Download: <${SITE}/download>`,
    `GitHub release: <${REPO}/releases/tag/v${version}>`,
    '',
    unwrap(body),
  ].join('\n');

  // The ping stands alone, ahead of the notes. Discord only notifies on the
  // message that carries the mention, so one short message pings once and the
  // notes arrive underneath it unencumbered — rather than an @everyone buried
  // in the first paragraph, or worse, one per chunk.
  const messages = [...(PING ? ['@everyone'] : []), ...chunk(document)];

  if (DRY) {
    console.log(`\n  ${version} — ${messages.length} message(s), nothing sent.\n`);
    messages.forEach((m, i) => {
      const pings = PING && i === 0 ? ', pings @everyone' : '';
      console.log(
        `  ---- message ${i + 1} of ${messages.length} (${m.length} chars${pings}) ----`,
      );
      console.log(m.replace(/^/gm, '  '));
      console.log('');
    });
    return;
  }

  const webhook = process.env.DISCORD_RELEASE_WEBHOOK;
  if (!webhook) {
    // GitHub will not hand a secret back once it is set, so there is nothing
    // to copy from the repository — the address has to come from Discord.
    die(
      'DISCORD_RELEASE_WEBHOOK is not set.\n' +
        '  In CI it comes from the repository secret of the same name; GitHub\n' +
        '  will not give that back, so to post by hand copy the webhook URL\n' +
        '  from the channel settings on Discord (Integrations, Webhooks).\n\n' +
        '  Or --dry-run here to see the messages without sending them.',
    );
  }

  for (const [i, message] of messages.entries()) {
    await post(webhook, message, PING && i === 0);
    console.log(`  posted ${i + 1} of ${messages.length}`);
    // Webhooks are rate-limited per channel; a short gap keeps order and
    // stays well clear of it.
    if (i < messages.length - 1) await new Promise((r) => setTimeout(r, 800));
  }
  console.log(`\n  Announced ${title} in Discord.\n`);
}

main().catch((e) => {
  console.error(`\n  Stopped: ${e.message}\n`);
  process.exit(1);
});

import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, extname, relative, resolve, sep } from "node:path";

const bookRoot = resolve(process.argv[2] ?? "target/mdbook");
const repositoryRoot = resolve(process.argv[3] ?? ".");

async function walk(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...await walk(path));
    else if (entry.isFile() && entry.name.endsWith(".html")) files.push(path);
  }
  return files;
}

function decode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

async function normalizeTarget(source, hrefPath) {
  let target = hrefPath ? resolve(dirname(source), decode(hrefPath)) : source;
  if (!target.startsWith(`${bookRoot}${sep}`) && target !== bookRoot) return target;
  try {
    if ((await stat(target)).isDirectory()) target = resolve(target, "index.html");
  } catch {
    if (!extname(target)) target = resolve(target, "index.html");
  }
  return target;
}

const files = await walk(bookRoot);
const documents = new Map();
for (const file of files) {
  const html = await readFile(file, "utf8");
  const ids = new Set(
    [...html.matchAll(/\bid\s*=\s*(["'])(.*?)\1/gi)].map((match) => decode(match[2])),
  );
  const hrefs = [...html.matchAll(/<a\b[^>]*\bhref\s*=\s*(["'])(.*?)\1/gi)]
    .map((match) => match[2]);
  documents.set(file, { hrefs, ids });
}

const broken = [];
const external = new Set();
let internalLinksChecked = 0;
for (const [source, document] of documents) {
  for (const rawHref of document.hrefs) {
    if (/^(?:https?:)?\/\//i.test(rawHref)) {
      external.add(rawHref);
      continue;
    }
    if (/^(?:mailto|tel|javascript|data):/i.test(rawHref)) continue;

    internalLinksChecked += 1;
    const [pathAndQuery, rawFragment = ""] = rawHref.split("#", 2);
    const hrefPath = pathAndQuery.split("?", 1)[0];
    const target = await normalizeTarget(source, hrefPath);
    const sourceName = relative(bookRoot, source).replaceAll(sep, "/");
    const targetName = relative(bookRoot, target).replaceAll(sep, "/");

    if (targetName.startsWith("../")) {
      broken.push({ source: sourceName, href: rawHref, reason: "outside build root" });
      continue;
    }

    let targetStats;
    try {
      targetStats = await stat(target);
    } catch {
      broken.push({ source: sourceName, href: rawHref, reason: `missing ${targetName}` });
      continue;
    }
    if (!targetStats.isFile()) {
      broken.push({ source: sourceName, href: rawHref, reason: `not a file ${targetName}` });
      continue;
    }

    if (rawFragment && target.endsWith(".html")) {
      const fragment = decode(rawFragment);
      if (!documents.get(target)?.ids.has(fragment)) {
        broken.push({
          source: sourceName,
          href: rawHref,
          reason: `missing anchor #${fragment} in ${targetName}`,
        });
      }
    }
  }
}

let repositoryLinksChecked = 0;
for (const href of external) {
  const url = new URL(href);
  const match = url.pathname.match(
    /^\/YanqingXu\/lua_rust\/(?:blob|edit|tree)\/main\/(.+)$/,
  );
  if (!match) continue;

  repositoryLinksChecked += 1;
  const target = resolve(repositoryRoot, decode(match[1]));
  try {
    await stat(target);
  } catch {
    broken.push({
      source: "external",
      href,
      reason: `missing repository path ${relative(repositoryRoot, target)}`,
    });
  }
}

console.log(
  `Checked ${files.length} pages, ${internalLinksChecked} internal links, `
  + `${external.size} external links, and ${repositoryLinksChecked} repository paths.`,
);
if (broken.length > 0) {
  console.error(`Found ${broken.length} broken links:`);
  for (const item of broken) {
    console.error(`- ${item.source}: ${item.href} (${item.reason})`);
  }
  process.exitCode = 1;
} else {
  console.log("Broken links: 0");
}

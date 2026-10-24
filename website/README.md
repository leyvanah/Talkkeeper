# Talkkeeper website

Static Russian-language product website for Talkkeeper. Built with Node.js
built-ins, with no dependency installation, analytics or external font requests.

## Files

- `*.mjs`: page templates, build settings and verified release details.
- `style.css`: shared page styles.
- `static/`: versioned CSS, browser scripts, logo and bundled font files.
- `tests/`: checks for paths, release information, themes and animation.
- `dist/`: generated output, ignored by Git; only this folder is deployed.

The interface illustration uses a fictional book-club conversation. Its tabs
switch reading views; audio, editing and model inference are not implemented
in the website. The hero animation uses one fixed grid of 54 vertical columns.

## Local preview

Requires Node.js 22 or later. From `website/`:

```sh
node build.mjs
node check.mjs
node serve.mjs
```

Open the loopback URL printed by the server. Local builds are not indexable.

To reproduce the GitHub Pages path locally:

```sh
SITE_URL=https://leyvanah.github.io/Talkkeeper/ node build.mjs
node check.mjs
node serve.mjs
```

On PowerShell, set variables using `$env:SITE_URL` before running the build.
The preview server reads the generated base path and serves `/Talkkeeper/`.

## Checks

```sh
node check.mjs
node tests/check-paths.mjs
node tests/check-release.mjs
node tests/check-theme.mjs
node tests/check-animation.mjs
```

`check.mjs` validates local links, anchors, font URLs, canonical URLs and the
sitemap against the actual output, including repository subpaths.

## GitHub Pages

In repository Settings > Pages, select **GitHub Actions** as the publishing
source. The owner or a maintainer must enable this once.

`.github/workflows/pages.yml` checks pull requests without publishing. After
changes under `website/` reach `main`, it builds and deploys only `website/dist`.
It never builds the desktop application. Official actions are pinned by commit.

The default public URL is `https://leyvanah.github.io/Talkkeeper/`.
`SITE_PUBLIC=1` enables indexing. `SITE_URL` supplies the HTTPS origin and optional
repository path. Both canonical URLs and static asset paths follow that setting.
For a custom domain, configure Pages and set the repository variable
`WEBSITE_URL` to the full HTTPS site URL before rebuilding.

The output includes `404.html`, `.nojekyll`, `sitemap.xml` and `robots.txt`.
For a project site, its `robots.txt` is under the project path; indexing control
also uses per-page robots metadata. The generated 404 remains non-indexable.

## Updating content and releases

Edit the templates or files under `static/`, then rebuild. Do not edit `dist/`.
`release.mjs` holds the verified Windows installer version, size and URL. Update
it when publishing a new application release. Installer files stay in GitHub
Releases and are not stored in this website folder.

The logo comes from Talkkeeper. Source Sans 3 is bundled under the SIL Open Font
License; its license is included alongside the fonts. Other website source is
covered by the repository's MIT license.

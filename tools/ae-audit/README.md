# AE import audit kit

One sitting inside After Effects, and docs/11's "verify against a live AE" rule is
satisfied for the whole effect table.

## Steps

1. Open After Effects (any version 2024+; no project needed).
2. If scripts cannot write files yet: Edit → Preferences → Scripting & Expressions →
   tick "Allow Scripts to Write Files and Access Network".
3. File → Scripts → Run Script File… → pick `audit.jsx` from this folder.
4. Wait for the alert (a minute or two). It writes `ae-audit-report.json` beside the
   script.
5. Hand the JSON back to the import work. Done.

## What it records

- Every effect the installation ships: match name, display name, category.
- For each of the 60 match names Lumit's import table claims
  (`claimed-matchnames.txt`): found / missing, the display name AE gives it, and its
  full property tree — each property's match name, name, value type and default.
  A missing name gets a best-guess suspect so a rename is one look to confirm.
- The AE version, so the audit is dated evidence rather than folklore.

The script builds one throwaway comp inside an undo group and removes it; an open
project is not modified.

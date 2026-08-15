---
type: bee.delivery
title: project-color-identity — delivery
description: "Delivery record for work item project-color-identity: 4 capped cell(s), 1 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: project-color-identity-delivery
  lifecycle: active
  areas: [bee-cockpit, appearance]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/appearance.md]
  sources: [.bee/cells/archive/project-color-identity/project-color-identity-1.json, .bee/cells/archive/project-color-identity/project-color-identity-2.json, .bee/cells/archive/project-color-identity/project-color-identity-3.json, .bee/cells/archive/project-color-identity/project-color-identity-4.json]
---

# project-color-identity — Delivery

## What shipped

A card on the Features board used to repeat its own feature name twice — once
as the heading and once as a small grey line beneath it — and then spend a row
of chips on facts the reader already had: which column the card was sitting in,
and which project it belonged to. The card now spends that space on what the
reader cannot otherwise see.

The line under the heading names the project and its working copy, separated by
a slash: the branch when the feature has its own working copy open, the word
`worktree` when the copy is open but its branch is not recorded, `merged` once
that copy has been folded back, and `Main` when the feature is worked in the
project's own checkout. On a board showing a single project the project half is
dropped, since every card there shares it; the line reads the feature name and
its working copy instead, and a feature whose record carries no title of its own
— whose heading is therefore already the feature name — keeps the line for the
working copy alone.

The chip row is gone. The column heading already names whether a feature is
waiting on the reader or in progress, and the working copy now has its own
place, so nothing in the row was still earning its space.

Every project draws in one fixed colour, painted as a stripe down the card's
left edge and as the colour of the project's name — on its cards and on its rows
in the Finished list alike. Two cards of the same project are recognisable at a
glance without reading either. A board showing a single project stays
uncoloured, since a colour that never varies distinguishes nothing.

Colours are handed out by position: the distinct projects appearing anywhere on
the board are put in order and given the palette's ten slots in turn. The first
attempt derived the colour from the project's name instead, and was replaced
after the running board proved it wrong — two of the three projects present drew
the same colour. Handing out slots by position cannot collide until an eleventh
project appears; the accepted cost is that registering a new project which sorts
ahead of others shifts their colours by one.

What was given up: the working-copy chip used to carry its state in its colour
as well as its words — blue for open, green for merged, grey for the main
checkout. On the new line the colour belongs to the project, so the state is
read from the words alone.

## Verify

`cargo test --workspace` green at 924, up from 916. New cases cover all four
working-copy spellings on both board kinds, the title-less card that keeps its
line for the working copy alone, the absence of the chip row, and — the case
that regressed — three real project names drawing three different colours, with
a project's colour proven identical on its cards and its Finished rows.

Confirmed against the running daemon after each change: the cross-project board
serves `beehive / wt/hold-holder-attribution` and `beedashboard / Main` in three
distinct colours, and a single project's board serves the feature name with its
working copy and no colour at all.

## Deviations

The work ran in the main checkout rather than its own branch checkout, for the
reason recorded against `card-badge-inside`: this session cannot write outside
the main checkout, so no worker could reach a branch checkout's files.

One worker died between committing its cell and reporting it. The commit and
the cap were both already on disk and the working tree matched them, so the
work was verified in place rather than redone.

## Provenance

Written at feature close from the four capped cell traces. The arrangement, the
removal of the chip row, and the retreat from name-derived colours to
positional ones are all recorded in the decision log.

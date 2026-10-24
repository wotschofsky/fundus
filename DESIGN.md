---
name: Transcript Vault Web
description: A minimalist white interface for a private recording archive.
colors:
  paper: "#fff"
  accent: "#18181b"
  accent-hover: "#343438"
  control-text: "#27272a"
  body: "#52525b"
  muted: "#71717a"
  line: "#e8e8ec"
  control-border: "#dedee3"
  control-border-hover: "#c9c9d0"
  subtle: "#f7f7f8"
  speaker: "#3f3f46"
  danger: "#b42318"
  error-border: "#f0c6c2"
typography:
  headline:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "28px"
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: "-0.035em"
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.5
    letterSpacing: "-0.015em"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.7
  transcript:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.85
  label:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "12px"
    fontWeight: 500
  metadata:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  chip: "3px"
  address: "4px"
  control: "5px"
spacing:
  tight: "6px"
  control-gap: "7px"
  compact: "12px"
  related: "16px"
  section: "30px"
components:
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.paper}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  button-primary-hover:
    backgroundColor: "{colors.accent-hover}"
  button-secondary:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.control-text}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  button-secondary-hover:
    backgroundColor: "{colors.subtle}"
  button-danger:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.danger}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  field:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.accent}"
    rounded: "{rounded.control}"
    padding: "9px 11px"
  navigation-item:
    textColor: "{colors.muted}"
  navigation-item-active:
    textColor: "{colors.accent}"
  chip:
    textColor: "{colors.body}"
    rounded: "{rounded.chip}"
    padding: "3px 6px"
  notice:
    textColor: "{colors.body}"
    rounded: "{rounded.control}"
    padding: "15px 16px"
  notice-error:
    textColor: "{colors.danger}"
  transcript-turn-active:
    backgroundColor: "{colors.subtle}"
    textColor: "{colors.control-text}"
    padding: "15px 0"
---

# Design System: Transcript Vault Web

## Overview

**Creative North Star: "Minimal white archive"**

The web interface is minimalist, modern, and white. Neutral text, thin dividers, plain horizontal navigation, and generous whitespace keep the recording list and transcript central. Page headings name the task directly; compact metadata supports the content without becoming ornament.

This records the shipped React web interface after the user's explicit white-interface direction superseded the earlier design. It establishes no native client tokens or architecture. The review screenshots contain synthetic demonstration content and provide web evidence only.

**Key Characteristics:**

- White page and control surfaces with neutral ink.
- Plain horizontal navigation that retains text labels on narrow screens.
- Divider-led rows with no decorative icon tiles or panels.
- Timestamp-linked audio beside spacious transcript prose.

## Colors

The palette is white and neutral gray, with red reserved for errors, failed processing, and destructive actions. The frontmatter is the normative reused color inventory.

### Primary

- **Neutral ink:** `accent` carries headings, selected navigation, primary button fills, checkbox selection, and the input caret. `accent-hover` is the primary button hover fill.

### Neutral

- **White:** `paper` is the page, field, button, and sticky audio-player ground.
- **Reading hierarchy:** `control-text` carries controls and transcript prose; `body` carries supporting text; `muted` carries metadata, hints, and unselected navigation. `speaker` strengthens speaker names and endpoint text.
- **Fine boundaries:** `line` separates archive rows and sections. `control-border` outlines fields and buttons; `control-border-hover` reinforces button hover. `subtle` provides secondary-button feedback and the currently playing transcript turn.
- **Error signal:** `danger` marks errors and destructive verbs; `error-border` outlines error notices without a filled color panel.

**The White Ground Rule.** Keep the archive on white; use neutral text and thin boundaries for hierarchy, with status color reserved for explicit problems and destructive actions.

## Typography

The application uses the platform system stack recorded in the frontmatter. No downloaded display face or separate brand font is required. Headings, controls, and prose share this stack; size, weight, spacing, and line height create the hierarchy.

- **Headline:** Desktop page headings use the headline role. Recording-detail headings use (30px, line-height 1.35), with a maximum width (760px). Both page and detail headings reduce to (25px) at the narrow breakpoint. Login headings use (26px).
- **Title:** Section headings use the title role. Recording-row titles use (17px, weight 500, letter-spacing -0.02em), reducing to (16px). Transcript section headings use (13px).
- **Body:** The body role describes inherited prose sizing. Library previews use (13px), reducing to (12px), clamp to two lines, and stop at (78ch). Summaries use (14px) at a maximum (72ch); search excerpts use (13px), clamp to five lines, and stop at (75ch).
- **Transcript:** Transcript prose uses the transcript role at both viewport sizes, preserves whitespace, and wraps long content.
- **Labels and metadata:** Labels use the label role. Dates, durations, and timestamps use tabular numerals. Recording metadata reduces to (10px) on narrow screens. Small text supports archive tasks rather than decorative overlines.

## Layout

A horizontal header spans the viewport at height (72px). Its horizontal padding is the greater of (28px) and half the space outside a (1000px) content span. The centered workspace has a maximum width (1056px), including padding (56px 28px 100px), yielding a desktop content span (1000px).

Page headings and actions share a simple row with a (40px) bottom margin. The toolbar holds the count and compact filters above the list's top divider. Recording rows use vertical padding (26px). The detail grid has a flexible reading column, a (180px) speaker/source column, and a (64px) gap. Settings sections stop at (680px).

At (760px), the header becomes (60px) tall with horizontal padding (20px), labeled navigation gaps reduce to (18px), and workspace padding becomes (34px 20px 70px). Detail content becomes a single column, with speakers below a divider. Filters wrap and token creation stacks. Recording rows use vertical padding (23px). At (350px), header padding and navigation gaps reduce further; workspace side padding becomes (16px). These are responsive web behaviors only.

The header scrolls with the page. The audio player is sticky at the viewport top, on white, with a bottom divider. Long titles, transcript prose, server addresses, and source filenames wrap within their containers. Empty source lists omit the Source recordings heading.

## Elevation & Depth

There are no decorative shadows, gradients, raised cards, or filled panels. White space and fine borders separate content. The audio player's stacking order supports reading and listening during scroll; its depth comes from a bottom divider. Only the currently playing transcript turn receives a very pale neutral fill.

**The Divider Rule.** Use whitespace and fine dividers to organize archive content; preserve an open reading surface instead of adding decorative containers.

## Shapes

Controls and notices have a small, functional control radius. Label chips use a smaller radius with a fine border and no fill. The server-address field uses the address radius. Archive rows, transcript turns, edit forms, summaries, and source sections keep an open rectangular structure. People use plain initial text rather than filled circular avatars.

## Components

### Buttons

Buttons have a fine border, white ground, medium weight (500), type size (12px), a minimum height (36px), and an internal icon/text gap (7px) where an icon is functional. Primary buttons use neutral ink with white text. Secondary hover adds a nearly white fill and a stronger border. Primary hover softens the ink fill. Disabled buttons use opacity (0.45) with a not-allowed cursor. Compact detail actions reduce to a minimum height (30px). Destructive actions use red text and explicit verbs; confirmation remains part of destructive behavior.

Keyboard focus uses the body gray in a (2px) outline with (3px) offset. This applies to links, buttons, inputs, and selects through the shared visible-focus selector.

### Inputs / Fields

Inputs and selects use white, a fine control border, the control radius, and minimum height (38px). Placeholder text uses muted gray. Search uses a borderless input above a bottom rule, with a separate explicit Search button. Focus retains the shared visible outline; there is no added colored border or glow. Errors use a readable outlined notice with an alert role rather than an invented field error style.

### Navigation

The header pairs the text brand Vault with labeled Recordings, Search, People, and Settings links. Desktop navigation gaps are (28px); each link spans the header height. The current destination uses neutral ink and a thin bottom border. Hover darkens link text. The same words remain visible on narrow screens; there is no icon-only navigation substitution.

### Chips

Labels are compact informational tags: type size (10px), a fine line border, and the chip tokens above. They wrap when needed, are hidden when empty, and are not interactive filter controls. Their purpose is archive metadata, not ornament.

### Cards / Containers

Archive and search results are plain linked rows separated by dividers. Metadata precedes a title and bounded preview. Recording and search-result hover underline the title; they do not fill the entire row. Notices are outlined on the white page. Error notices substitute red text and an error border. Edit forms use top and bottom rules rather than a filled panel. Empty states are plain headings and explanatory text with no decorative icon.

### Timestamp-linked transcript

Turns place a text timestamp beside the speaker and passage. The timestamp column is (50px) with a (16px) gap; narrow screens use (42px) and (12px). Timestamps use (10px) tabular figures and a minimum button height (32px); the play glyph is hidden. Clicking a timestamp seeks the original audio. The active turn receives the subtle fill only while playback is active. Unavailable audio disables timestamp actions. Optional following scrolls the playing turn into view. Playback retains the browser-native audio control.

### Loading and motion

Loading uses a status label and a rotating functional indicator (1s linear, repeating). The interface adds no decorative motion or row-hover transition. Reduced-motion preferences disable animations, transitions, and smooth scrolling through the shared override.

## Do's and Don'ts

### Do:

- **Do** keep white as the page and control ground, using neutral type hierarchy and fine dividers.
- **Do** retain labeled navigation and visible keyboard focus on narrow web screens.
- **Do** preserve bounded, wrapping transcript prose and timestamp access to original audio.
- **Do** make errors, failed processing, and destructive actions explicit in words as well as color.

### Don't:

- **Don't** reintroduce warm backgrounds, green accents, decorative icon tiles, colored panels, gradients, or shadows.
- **Don't** replace open archive rows with ornamental cards or badges.
- **Don't** treat responsive web screenshots as native client or physical hardware validation.

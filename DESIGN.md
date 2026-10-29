---
name: Fundus Web
description: A minimalist white interface for a private recording archive.
colors:
  paper: '#fff'
  accent: '#18181b'
  accent-hover: '#343438'
  control-text: '#27272a'
  body: '#52525b'
  muted: '#71717a'
  line: '#e8e8ec'
  control-border: '#dedee3'
  control-border-hover: '#c9c9d0'
  subtle: '#f5f5f6'
  speaker: '#3f3f46'
  danger: '#b42318'
  error-border: '#f0c6c2'
typography:
  headline:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '28px'
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: '-0.03em'
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '15px'
    fontWeight: 600
    lineHeight: 1.5
    letterSpacing: '-0.01em'
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '14px'
    fontWeight: 400
    lineHeight: 1.65
  transcript:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '15px'
    fontWeight: 400
    lineHeight: 1.7
  label:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '13px'
    fontWeight: 500
  metadata:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: '12px'
    fontWeight: 400
    lineHeight: 1.5
rounded:
  chip: '4px'
  control: '6px'
  dialog: '10px'
spacing:
  tight: '6px'
  control-gap: '8px'
  compact: '12px'
  related: '16px'
  section: '28px'
components:
  button-primary:
    backgroundColor: '{colors.accent}'
    textColor: '{colors.paper}'
    rounded: '{rounded.control}'
    padding: '7px 12px'
  button-primary-hover:
    backgroundColor: '{colors.accent-hover}'
  button-secondary:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.control-text}'
    rounded: '{rounded.control}'
    padding: '7px 12px'
  button-secondary-hover:
    backgroundColor: '{colors.subtle}'
  button-danger:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.danger}'
    rounded: '{rounded.control}'
    padding: '7px 12px'
  field:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.accent}'
    rounded: '{rounded.control}'
    padding: '7px 10px'
  navigation-item:
    textColor: '{colors.muted}'
  navigation-item-active:
    textColor: '{colors.accent}'
  chip:
    textColor: '{colors.body}'
    rounded: '{rounded.chip}'
    padding: '2px 7px'
  notice:
    textColor: '{colors.body}'
    rounded: '{rounded.control}'
    padding: '14px 16px'
  notice-error:
    textColor: '{colors.danger}'
  transcript-turn-active:
    backgroundColor: '{colors.subtle}'
    textColor: '{colors.control-text}'
    rounded: '{rounded.control}'
    padding: '12px'
  dialog:
    backgroundColor: '{colors.paper}'
    rounded: '{rounded.dialog}'
    padding: '24px'
---

# Design System: Fundus Web

## Overview

**Creative North Star: "Minimal white archive"**

The web interface is minimalist, modern, and white. Neutral text, thin dividers, plain horizontal navigation, and generous whitespace keep the recording list and transcript central. Page headings name the task directly; compact metadata supports the content without becoming ornament.

This records the shipped Svelte web interface after the user's explicit white-interface direction superseded the earlier design. It establishes no native client tokens or architecture. Review screenshots use synthetic demonstration content and provide web evidence only.

**Key Characteristics:**

- White page and control surfaces with neutral ink.
- Plain horizontal navigation that keeps its text labels on narrow screens.
- Divider-led rows grouped under quiet day headings; no decorative icon tiles or panels.
- Timestamp-linked audio beside spacious transcript prose.

## Colors

The palette is white and neutral gray, with red reserved for errors, failed processing, and destructive actions. The frontmatter is the normative reused color inventory; `style.css` declares the same values as custom properties on `:root`.

### Primary

- **Neutral ink:** `accent` carries headings, selected navigation, primary button fills, checkbox and slider selection, and the input caret. `accent-hover` is the primary button hover fill.

### Neutral

- **White:** `paper` is the page, field, button, dialog, and sticky audio-player ground.
- **Reading hierarchy:** `control-text` carries controls and transcript prose; `body` carries supporting text and day headings; `muted` carries metadata, hints, and unselected navigation. `speaker` strengthens speaker names.
- **Fine boundaries:** `line` separates archive rows and sections. `control-border` outlines fields and buttons; `control-border-hover` reinforces button hover. `subtle` provides secondary-button feedback and the current transcript turn.
- **Error signal:** `danger` marks errors, failed processing, and destructive verbs; `error-border` outlines error notices and destructive-button hover without a filled panel. Only the confirming button inside a destructive dialog is filled red.

**Dark mode** follows the system setting (`prefers-color-scheme`); there is no toggle. A dark set of the same custom properties keeps the light mode's contrast: a near-black ground, light ink, zinc grays, a lighter red for `danger`, and dark text on filled buttons. The dialog drops its shadow for a fine `line` border; native controls, scrollbars and the audio player follow through `color-scheme`.

**The White Ground Rule.** Keep the archive on white; use neutral text and thin boundaries for hierarchy, with status color reserved for explicit problems and destructive actions.

## Typography

The application uses the platform system stack recorded in the frontmatter. No downloaded face is required. Size, weight, spacing, and line height create the hierarchy.

- **Floor:** no text is smaller than (12px). Metadata, chips, timestamps, hints, and counters use (12px); labels, buttons, navigation, and helper text use (13px).
- **Headline:** Page headings use the headline role; recording-detail headings use (30px, line-height 1.3, max width 760px). Both reduce to (24px) at the narrow breakpoint. Login headings use (26px).
- **Title:** Section headings use the title role. Archive and search-result titles use (16px, weight 500, letter-spacing -0.015em). Day headings, transcript and side-panel headings use (13px, weight 600). Settings section headings use (18px).
- **Body:** Library previews use (14px), clamp to two lines, and stop at (78ch). Summaries use (15px) at (72ch); search excerpts use (14px) and clamp to four lines, with the query's words emphasized by weight (`mark`, no fill).
- **Transcript:** Transcript prose uses the transcript role at every width, preserves whitespace, wraps long content, and stops at (72ch).
- **Numerals:** dates, durations, timestamps, and slider values use tabular figures. Dates follow the viewer's locale and time zone.

## Layout

A horizontal header spans the viewport at height (64px); its horizontal padding is the greater of (28px) and half the space outside a (1000px) content span. The centered workspace has a maximum width (1056px), including padding (48px 28px 96px).

Page headings and actions share a row with a (28px) bottom margin. The library toolbar holds the count and compact filters. Recordings are grouped by calendar day under a (13px) heading with a bottom divider; rows use vertical padding (20px). The detail grid has a flexible reading column, a (220px) speaker/source column that stays in view while the transcript scrolls, and a (56px) gap. Settings sections stop at (680px).

At (760px) the header becomes (56px) tall with padding (20px); workspace padding becomes (28px 20px 72px). The detail grid becomes one column and the speaker/source panel moves **above** the transcript, so naming speakers never requires scrolling past a long recording. Filters wrap, token creation and row actions stack. At (360px) header padding and navigation gaps reduce further; workspace side padding becomes (16px).

The header scrolls with the page. The audio player is sticky at the viewport top, on white, with a bottom divider. Navigating to a page starts at its top or at its `#anchor`; Back and Forward return to the previous scroll position, and list filters live in the URL. Long titles, transcript prose, addresses, and filenames wrap within their containers.

## Elevation & Depth

There are no decorative shadows, gradients, raised cards, or filled panels. White space and fine borders separate content. The one elevated surface is the confirmation dialog: a soft shadow over a translucent ink backdrop, no border (in dark mode a fine border instead of the shadow). Only the current transcript turn receives a pale neutral fill.

**The Divider Rule.** Use whitespace and fine dividers to organize archive content; preserve an open reading surface instead of adding decorative containers.

## Shapes

Controls, notices, and the active transcript turn use the control radius (6px). Label chips use (4px) with a fine border and no fill. The dialog uses (10px). Archive rows, edit forms, summaries, and source lists keep an open rectangular structure. People are plain text rows, without avatars.

## Components

### Buttons

Buttons have a fine border, white ground, weight (500), type size (13px), minimum height (34px), and an icon/text gap (6px) where an icon is functional. Primary buttons use neutral ink with white text; hover softens the fill. Secondary hover adds the subtle fill and a stronger border. Disabled buttons use opacity (0.45) with a not-allowed cursor. Compact detail actions reduce to (30px). Destructive buttons use red text and explicit verbs, and always ask through a dialog first. Text buttons (underlined, no border) carry low-emphasis inline actions such as confirming a voice match.

Keyboard focus is a (2px) body-gray outline with (2px) offset on every link, button, field, and select.

### Inputs / Fields

Fields and selects use white, a fine control border, the control radius, and minimum height (36px). Placeholder text uses a gray with readable contrast. Search is a borderless input above a bottom rule that darkens while focused, with a separate Search button and no browser clear glyph. Recording time is edited as a native date-time field in the viewer's local time; only a changed value is sent. Voice thresholds are native range sliders with their value beside them. Errors use an outlined notice with an alert role.

### Navigation

The header pairs the text brand Fundus with labeled Recordings, Search, People, and Settings links; gaps are (26px). The current destination uses neutral ink and a thin bottom border. The same words remain on narrow screens. The library header offers quiet text links to Trash and Connect apps. Keyboard: `/` opens or focuses search; on a recording, Space plays or pauses unless a control has focus.

### Chips

Labels are informational tags: (12px) text, fine border, chip radius. They wrap, are omitted when empty, and are not interactive.

### Rows and containers

Archive, trash, people, token, connection, and search rows are plain rows separated by dividers. Metadata (time, duration, processing state with a small spinner) precedes the title and bounded preview. Linked rows underline the title on hover; they do not fill. Notices are outlined; the processing notice pairs the state with Resume or Retry once stopped; Move to Trash cancels a running one. Edit forms and inline renames replace the row in place between rules rather than opening a panel. Empty states are a plain heading, a sentence, and at most one action.

### Dialog

Confirmations use the native modal `<dialog>`: title, one explanatory sentence, Cancel, and an explicit action verb. Escape and Cancel close it unless the action is running; failures show inside the dialog.

### Timestamp-linked transcript

Turns place a (12px) tabular timestamp beside the passage in a (52px) column with a (14px) gap ((44px) and (10px) narrow). The speaker name appears only when the speaker changes; continued turns tighten their top padding. Clicking a timestamp seeks and plays. The turn being spoken (or opened from a search result) receives the subtle fill and dark timestamp. Unidentified speakers are numbered in speaker-list order ("Speaker 1"). Optional following scrolls the playing turn into view. Playback keeps the browser-native audio control.

Speakers are assigned with a native select that also offers "New person…", which turns into an inline name field. Voice matches can be confirmed in one step, and quoted self-introductions offer "This is …".

### Loading and motion

Loading uses a status label with a rotating indicator (1s linear). There is no decorative motion. Reduced-motion preferences disable animation, transitions, and smooth scrolling.

### Performance notes

The library polls only while a visible recording is processing; a recording polls its status only while processing and refetches its transcript only when the stage changes. The full transcript loads page by page in the background. Turns are memoized and use `content-visibility: auto`, and playback updates state only when the spoken segment changes, so playback cost does not grow with transcript length.

## Do's and Don'ts

### Do:

- **Do** keep white as the page and control ground, using neutral type hierarchy and fine dividers.
- **Do** keep labeled navigation and visible keyboard focus on narrow screens.
- **Do** keep transcript prose bounded and wrapping, with timestamp access to the original audio.
- **Do** make errors, failed processing, and destructive actions explicit in words as well as color, and confirm destructive actions in a dialog.

### Don't:

- **Don't** reintroduce warm backgrounds, green accents, decorative icon tiles, colored panels, gradients, or decorative shadows.
- **Don't** replace open archive rows with ornamental cards or badges.
- **Don't** use browser `prompt`/`confirm`, or text smaller than 12px.
- **Don't** treat responsive web screenshots as native client or physical hardware validation.

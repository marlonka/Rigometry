# Interface design system

Reusable visual, interaction and writing rules for native apps and websites. Choose structure from the user's task; no product, domain, framework or component library is assumed.

Rules describe required behavior. Numeric baselines are starting values, expressed in logical/CSS pixels rather than physical screen pixels. Adapt dimensions to content, platform and accessibility settings. Record product-specific exceptions separately; keep this file portable.

## Principles

- Use near-black or off-white surfaces, restrained neutral layers and readable typography.
- Establish hierarchy through grouping, alignment, spacing and contrast before adding containers.
- Keep navigation and controls monochrome. Reserve color for meaningful states, distinguishable data series and actual source content.
- Keep geometry stable through hover, press, selection, loading and live updates.
- Show frequent actions directly; disclose advanced details next to the decision they support.
- Preserve input, selection, filters, focus and scroll position through ordinary updates.
- Write only actions, identities, concrete facts, values and decision-relevant states.
- Distinguish unknown, loading, unavailable, failed, empty, zero and complete.
- Use native semantics and controls where possible. Keyboard, zoom, touch and reduced motion are part of the design.

Avoid gradients, glass, glow, background blobs, oversized decorative headings, cards around every row, animated selection pills and button hover lifts. Distinctiveness comes from proportion, composition and behavior.

## Layout and scrolling

Choose the simplest complete workflow: a focused form, reading column, list, editor, collection or work area. A dashboard and sidebar are optional patterns, never starting requirements.

| Property | Baseline |
| --- | --- |
| Spacing scale | 4, 8, 12, 16, 20, 24, 32, 40, 48, 64 |
| Related controls | 8–12 |
| Surface padding | 16–24 |
| Separation between sections | 24–32 |
| Desktop page inset | 24–40 |
| Narrow page inset | 12–16 |
| Focused form width | Up to 768 |
| Reading line length | About 60–80 characters |
| Standard control height | 40 |
| Primary touch target | At least 44 × 44 |
| Ordinary divider | 1 |

Align related page titles, toolbars, sections and tables to shared edges. Wide windows may add useful working space or margins; do not stretch a short input, label column or paragraph across the entire screen.

### One vertical scroll owner

- Let ordinary cards, small tables, drive lists, property lists and grouped settings grow with their content. Scroll the page.
- Do not put a fixed-height scrolling box inside an already scrolling page just to make cards equal in height.
- Independent scroll regions need a real task: an editor, long picker, navigation tree, virtualized dataset or persistent inspector.
- Virtualization is a rendering strategy, not a reason to add another scrollbar. Prefer the page's scroll owner when the library supports it.
- If a bounded dataset needs internal scrolling, make it a deliberate work area with a visible boundary, enough useful rows, keyboard access and predictable wheel behavior.
- Horizontal scrolling belongs only to genuinely two-dimensional content when reflow would lose meaning. Never let a page overflow because a child refuses to shrink.
- Reserve scrollbar space before measuring content. Leave clearance between the last value and the track, including overlay scrollbars and wide accessibility scrollbars.
- Scrollbar appearance, hover expansion and modal scroll locking must not shift the content horizontally.
- Sticky headers and footers reserve space; the last item and focused control stay reachable.

### Responsive composition

Use the available container width, not the screen width, to decide whether columns fit. Stack sections before names, values or actions become cramped. Wrap long names; keep a full value reachable without relying on hover.

For web layouts, use `min-width: 0`, `minmax(0, 1fr)` and stable scrollbar gutters where appropriate. For native toolkits, constrain each child to the allocated content rectangle and account for logical scale and system DPI.

Support narrow layouts and increased text size without losing actions. On the web, verify reflow at 320 CSS px; for native apps, verify the advertised minimum window size and maximum supported scale. Do not make users zoom out to finish a task.

### Action groups and navigation

- Size an input and its adjacent actions as a group. Reserve space for every relevant state, including cancel or retry.
- Keep a primary action and its cancel/stop action adjacent. On a narrow layout, move the entire action group to one intentional row; do not strand Cancel below a full-width field.
- Give short inputs a sensible maximum width. Long paths may use the remaining width after actions have been measured.
- Reserve stable slots for changing button labels and busy icons. Localized text must fit.
- Sidebar sections use layout flow with a flexible spacer, not absolute offsets that collide at increased scale.
- Keep branding compact. Put lower-priority destinations below primary navigation without adding routine diagnostic status text.
- Keep page title and object identity distinct; avoid repeating the same title in multiple containers.

## Neutral tokens

Components consume semantic roles. Keep dark and light values together.

| Role | Dark | Light |
| --- | --- | --- |
| Page | #0E0E0E | #F7F7F7 |
| Surface | #171717 | #FFFFFF |
| Raised surface | #222222 | #F0F0F0 |
| Hover | #292929 | #EAEAEA |
| Selected | #303030 | #E2E2E2 |
| Primary text | #EEEEEE | #161616 |
| Secondary text | #B4B4B4 | #595959 |
| Decorative divider | #303030 | #DDDDDD |
| Essential control border | #777777 | #808080 |
| Primary action background | #EEEEEE | #161616 |
| Primary action text | #111111 | #FFFFFF |
| Focus ring | #B4B4B4 | #535353 |

These values are starting tokens, not automatic accessibility certification. Measure actual adjacent colors and composited states. Decorative dividers may be subtle; essential boundaries need sufficient contrast.

Use state color locally, paired with text or a recognizable symbol. Do not tint an entire page to indicate one problem. Preserve original colors in photos, documents and other source media.

Use one radius family: 8–10 for controls, 12–16 for menus and compact groups, 16–24 for substantial surfaces. Reduce inner radii with the inset. Ordinary cards do not need shadows; floating menus and dialogs may.

Light mode needs its own hierarchy and contrast checks. Keep geometry and semantics identical across themes. Resolve the saved/system theme before first paint; avoid flashing the opposite theme.

## Typography and icons

Use a neutral sans-serif with open forms and appropriate language coverage. A system stack is often sufficient. Keep font loading from moving the layout.

| Role | Starting size | Weight |
| --- | --- | --- |
| Page title | 26–30 | 500 |
| Section title | 18–20 | 500 |
| Body and field input | 16 | 400 |
| Navigation and controls | 15–16 | 400–500 |
| Secondary metadata | 13–14 | 400 |
| Primary metric | 28–32 | 400–500 |

Default scale must be comfortable on a normal desktop display. Empty space is not a reason to make text smaller. Secondary text is lower priority, not illegible. Allow user scaling independently of the default baseline; migrate stored preferences deliberately if the baseline changes.

Keep ordinary content left-aligned, labels close to values and numbers aligned for comparison. Do not justify short labels or spread letters to fill columns. Wrap a long label naturally.

Use one icon family with a shared viewbox, optical size, stroke and rounded joins. Typical navigation glyphs are 20–24; use about 1.5–2 units of stroke in a 24-unit viewbox. Avoid unnecessary miniature detail.

Disclosure arrows must read as controls: an approximately 16-unit chevron inside a generous click target, a visible hover/focus state and keyboard expansion. Keep the glyph slot fixed when it rotates.

A brand mark identifies the product; a navigation icon identifies a destination. They need not be the same drawing. Use vector masters and inspect raster exports at actual 16, 24, 32 and 48 px sizes as well as enlarged scale. Do not claim uniqueness from original authorship alone.

## Metrics, tables and live data

- Use tabular numerals for changing values and aligned numeric columns.
- Keep number and unit together with a small gap. Reserve width around the complete pair, not between the digits and their unit.
- Group closely related readings on the same stable header row when they fit. Stack coherently at narrow widths.
- Keep row identity and order stable during ordinary updates. Do not automatically reorder a table under the pointer unless the user requested a live sort.
- Bound numeric columns for expected precision and unavailable states. Long errors belong in details, not in a value slot that stretches the page.
- Keep comparisons aligned with consistent units, precision and explicit scope.
- Present user-relevant data first. Acquisition frequency, internal counters, raw timestamps and provider details belong in diagnostics unless they affect an immediate decision.
- Firmware/configuration placeholders are missing information, not measurements. Do not guess a manufacturer, revision or value from appearance.
- Choose tables for repeated comparable fields and property lists for heterogeneous details. Do not put every property in its own card.

Separate acquisition from rendering. New samples must not reconstruct the page or reset focus and scrolling. Update only the affected values and geometry.

Charts use observed timestamps and preserve gaps. Smooth panning or short visual interpolation can reduce abrupt motion, but must not invent observations or change displayed/exported measurements. Avoid spline overshoot that suggests values outside the source range. Keep scales stable or change them deliberately.

Pause unnecessary animation off-screen. Respect reduced motion during the session. A stopped or failed stream must not keep looking fresh.

## Components and states

### Controls and forms

Use one clear primary action per task region. Secondary actions remain quieter. Keep the box, border width and font metrics unchanged across states; focus outlines must not alter layout.

Labels remain visible. Placeholders show examples, not the only label. Explain relevant constraints before submission. Retain exact input after failure; never silently truncate or replace an intentional blank.

Prefer undo for reliably reversible changes. Confirm consequential irreversible operations with the exact object and effect. Do not ask for confirmation on every routine action.

### Lists, tabs and disclosure

Tabs switch views of one context; filters change membership; navigation changes location. Match semantics and keyboard behavior to that distinction.

Selection appears in place. Do not animate a shared highlight across tabs. Keep filters and return position when inspecting an item. Expanding a disclosure must not unexpectedly collapse unrelated content above it.

Rows can have separate selection, expansion and action controls. Give each a distinct hit target; avoid nested buttons or ambiguous double-click-only functionality.

### Dialogs, menus and detail panes

Open dialogs and menus at their final position and size. Constrain them to the viewport. A long picker may scroll; ordinary page cards should not imitate it.

Dialogs have a name, meaningful initial focus, reachable cancel/close, appropriate focus containment and focus restoration. Escape dismisses the top applicable layer. Errors persist where recovery happens; a disappearing toast cannot be their only presentation.

Use an inspector only when simultaneous context is useful. Reserve its final layout once, then animate only its inner surface if motion helps orientation. Preserve the surrounding page's reading position.

## Motion

Use short, interruptible transitions. Most controls stay still. Motion communicates a state change; it does not decorate every interaction.

| Preset | Duration | Easing | Use |
| --- | --- | --- | --- |
| Press | 60 ms | ease-out | Tone or opacity feedback |
| Release / color | 140 ms | ease-out | Resting state, border or background |
| Local fade | 120 ms | ease-out | Local content/state |
| Enter | 160 ms | cubic-bezier(0.2, 0, 0, 1) | Floating surface |
| Exit | 100 ms | cubic-bezier(0.4, 0, 1, 1) | Floating surface removal |
| Page | 180 ms | cubic-bezier(0.2, 0, 0, 1) | Content at its final position |

Optional springs, in a mass/stiffness/damping convention: control rotation 0.8/500/38; detail-pane entrance 0.9/400/38. Translate parameters for the chosen engine. Avoid conspicuous bounce.

Do not scale buttons, lift cards on hover, slide ordinary pages or animate routine width/height/padding changes. Apply intentional structural changes once. Avoid `transition: all`.

Commit state immediately. Retarget interrupted transitions from their current state; never queue obsolete actions. Remove exiting content from interaction and keyboard focus immediately. Essential logic must not wait for animation completion.

Reduced motion uses immediate state changes, no pane slide, no decorative loops and no smooth scrolling. Keep final values, progress and icon orientation correct. Direct dragging, scrubbing and resizing follow input without spring delay.

## Writing

Every text element must communicate an action, identity, concrete fact, value, relevant state or decision. Empty space does not require copy.

Use specific verbs and consistent nouns. A visible action rarely needs a second sentence explaining itself. Remove routine implementation information from the primary interface.

Show failures near the affected field or task, with enough information to recover. Missing information can be concise in the main view and explained in details. Do not invent certainty, progress, saved state or successful verification.

No slogans, brand poetry, invented testimonials, motivational filler or decorative headlines. Warnings belong where their consequence matters, not under every section.

## Accessibility and verification

Use semantic HTML or the native toolkit's accessibility roles. Give icon-only controls names. Preserve reading order, focus order and keyboard alternatives to custom charts/maps.

Measure contrast: 4.5:1 for ordinary text, 3:1 for qualifying large text and essential non-text cues. Color cannot be the only status signal. Check focus on selected rows and high-contrast themes.

Test increased text size, translated labels and long data before fixing heights. On touch, aim for 44 × 44 targets; meet platform/accessibility minimums for secondary targets. Tooltips cannot contain the only accessible full value or action.

Before delivery:

1. Complete the main workflow using realistic populated data, empty data and a meaningful failure.
2. Inspect both themes at normal and compact widths, default and increased scale.
3. Scroll to the bottom of every affected section. Check the rightmost values, scrollbar clearance and sticky controls.
4. Exercise keyboard navigation, focus restoration, cancellation and retry.
5. Observe repeated live updates. Values, units, action rows and nearby controls must not drift.
6. Test rapid open/close/reopen and reduced motion.
7. Run existing build and behavior checks. Add tests for meaningful regressions, not copies of style constants.

Measure geometry through the interaction where stability matters. In web interfaces, compare bounding rectangles on successive animation frames; roughly 0.5 logical px is a useful tolerance, excluding intentional scroll/resize. Use the equivalent toolkit measurements in native apps.

A final screenshot does not prove update stability or accessibility. A passing automated check does not replace reading the screen and completing the task.

## Reuse

Copy this file into another project. Identify its users, primary task, content, required states, navigation and platform constraints. Apply only the patterns that support that task. Keep product vocabulary, release history and verification logs outside this guide.

# Design & UI notes

Practical notes for working on the Warp client UI in this fork. Add durable,
non-obvious findings here (things that cost time to rediscover), not a full style
guide.

## Fonts in the composer / input chrome

The terminal grid font and most input chrome are both derived from a single
setting: `appearance.monospace_font_size()` (default in
`app/src/settings/font.rs`). This fork renders the terminal grid a couple points
smaller, so composer chrome that tracks `monospace_font_size()` is compensated
back up to its intended size in a few specific places rather than globally:

- `app/src/context_chips/display_chip.rs` — `udi_font_size()` / `udi_icon_size()`
  size the UDI composer chips (dir, `aA`, model/profile selector, remote-control).
- `app/src/view_components/action_button.rs` — `ButtonSize::{UDIButton,
  UDIPromptChip, AgentInputButton}` in `font_size()` / `icon_size()` size the
  footer buttons and chip labels.

If a composer element looks "stuck small no matter what you change," check which
`ButtonSize` / helper actually feeds it. The visible bottom-row chips
(`Default | Bang`, `/remote-control`, dir) route through
`ButtonSize::UDIButton` / `UDIPromptChip` (see `profile_model_selector.rs`,
`universal_developer_input.rs`) — a different knob than `udi_font_size()`.

## Sizing icons on icon-only `ActionButton`s (mic, lightning, `+`)

Non-obvious and easy to get wrong: for an **icon-only** `ActionButton`
(`icon.is_some() && label.is_empty()`), the button renders as a **square of
`ButtonSize::button_height`** and the glyph is centered inside it (see
`ActionButton`'s `View` impl in `app/src/view_components/action_button.rs`, the
`is_icon_only()` branch).

- The glyph itself is sized by `ButtonSize::icon_size()`, but it is **capped by
  the surrounding `button_height` square**. If `icon_size <= button_height` the
  glyph renders at `icon_size`; otherwise it's clamped to the square.
- `icon_size()` had historically been computed from
  `font_cache().line_height(font_size, DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4)`,
  which lands at only ~60% of the button square. As a result, `±1–2pt` tweaks to
  `icon_size` are visually imperceptible — the glyph barely moves.
- **To make an icon-only footer glyph visibly larger, size it as a fraction of
  the button square**, e.g. `ButtonSize::AgentInputButton => self.button_height(
  appearance, app) * 0.8`. This enlarges the glyph without changing the button
  (so it doesn't grow the surrounding row height).

Corollary: to make the button *row* tighter without shrinking the glyph, shrink
`button_height` and raise the icon fraction so the net glyph size holds.

## Icon-only button hit target / clickability

Actions dispatched by an `ActionButton` route through the **view ancestry**, not
the rendered element tree. If a footer button is rendered as a `ChildView` by a
view that does not own it, `report_view_embeddings` reparents it to the rendering
view and its typed action is silently dropped (button looks fine, does nothing).
Keep footer buttons parented to the view that handles their action (e.g. render
`AgentInputFooter` as a single `ChildView` rather than pulling its buttons out
inline). See `app/src/terminal/input/agent.rs` and
`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`.

## Floating input box (agent / terminal / cloud composer)

Shared "floating rounded box" chrome lives in
`app/src/terminal/input/common.rs` (`floating_input_box`,
`FLOATING_INPUT_CORNER_RADIUS`, `FLOATING_INPUT_MARGIN`). Mode-specific
padding/background is applied by the caller before `.finish()`
(`render_agent_input` in `app/src/terminal/input/agent.rs`, plus the terminal /
cloud-agent variants). In agent/UDI mode the editor's own top margin and bottom
padding are `0` (`terminal_input_spacing`), so the box's vertical extent is
driven mostly by the composer row's control heights plus the box's explicit
`with_padding_*` — tune those, not the editor.

## Inline menu font sizing (slash commands, models, prompts, etc.)

The dropdown menus that appear above the composer (`/COMMANDS`, model/profile
selector, `@` file paths, prompts, skills, history, repos, plans, conversations)
all share `app/src/terminal/input/inline_menu/styles.rs`:

- `font_size()` sizes the **rows** (name + description). The fork shrank the base
  monospace font, so this was bumped from `monospace - 2` to `monospace`.
- The **row height** is a *separate* constant: `result_item_height_fn` in
  `inline_menu/view.rs` (`QUERY_RESULT_RENDERER_STYLES`). It does **not** track
  `font_size()` automatically. If you bump the row font without bumping this, the
  taller glyphs clip against the fixed-height row and the text renders **blank**
  (the classic "fonts went missing" symptom). Keep them in step (`monospace + 10`
  for the current `monospace` row font).

The nav/hint bar at the bottom (`↑ ↓ to navigate`, `esc to dismiss`) is a
**different** element — a `Message` rendered via
`message_bar/common.rs::render_standard_message_bar`, shared with the
terminal/agent status bars. To enlarge only the inline-menu nav bar (not the
other bars):

- `inline_menu/styles.rs::message_bar_font_size()` = row font + 2.
- `render_standard_message_bar_with_font(msg, right, Some(font), app)` threads the
  override through `render_message_bar_items` (text/hyperlink/icon) and into the
  keystroke chips via `render_keystroke_with_color_and_font_overrides` (the `↑ ↓
  esc` caps have their **own** font, `shortcuts::styles::font_size`, so text-only
  overrides leave the caps small). Default callers pass `None` and are unchanged.
- The menu's reserved height is computed in `inline_menu/positioning.rs`
  (`menu_frame_height`) via `standard_message_bar_height_with_font`; it must use
  the same font as the bar or the frame height desyncs from the rendered bar.

## SVG icon color / masking

Bundled SVG icons are rasterized and then tinted by the shader using the red
channel as an alpha mask. Visible parts of an icon SVG must use a high-red fill
(e.g. `fill="white"` / `stroke="white"`), not `black`, or they render invisible.
See folder glyphs in `app/assets/bundled/svg/`.

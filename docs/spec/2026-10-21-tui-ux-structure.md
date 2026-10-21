# TUI UX structure (B82) — overlay stack, shared geometry, key map

- Date: 2026-10-21
- Status: Shipped (2026-10-21); K1–K5 accepted. §3 superseded in B83: instead of per-tab `layout()` functions, renderers record the rects they draw into a per-frame `HitMap`, which gives the same no-drift guarantee without moving rect math out of the renderers
- Backlog: B82 (prerequisite for B83 mouse)
- Source: [audit 2026-10-21](../audit/2026-10-21-tui-mouse-ux-assessment.md) U1–U8, plus items deferred
  from B79 (confirm replaces body) and B80 (context-aware hints).

## 1. Goals / non-goals

Goals: one place that knows which overlay is on top; one geometry function per screen region that
render and input (later: mouse) both call; a key map without cross-tab overloads; less chrome.

Non-goals: mouse (B83), `/` filter, toasts, new operations, theme changes.

## 2. Overlay stack (U3, B79 leftover)

Today ~13 overlays live in scattered fields (`PopupState.{auth, export, member_picker}`,
`help_open`/`help_section`, info popup, `log_overlay_open`, results popup, running-op,
`ConfigTab.{entry_form, confirm, editing_field, direct_vec_editor, direct_group_picker}`, Operate/View
text-input modes). Their priority exists only as the `if` order in `handle_popup_key`.

Design:
- `enum Overlay { Auth, Export, MemberPicker, Help, Info, Log, Results, Confirm, EntryForm,
  FieldEdit, VecEditor, GroupPicker }`; overlay *state* stays where it lives today (no data move).
- `fn App::overlays(&self) -> Vec<Overlay>` derives the stack bottom→top from existing state in one
  function, which documents and fixes the priority. Auth always top.
- Input: `handle_popup_key` dispatches to `overlays().last()` only — a modal consumes all events.
- Render: draw the base screen **always**, then each overlay in stack order. Config confirm and
  entry form become true overlays over the Config body (fixes "confirm blanks the body").
- Inline text editing (Operate/View inputs) is *not* an overlay; it is a focus mode (§5).

## 3. Shared geometry

- Per-tab `fn layout(area, &state) -> TabLayout` (plain struct of `Rect`s, saturating math) used by
  both `render_*` and any input code that needs positions (Operate scroll from B78, viewports'
  `visible_height`). Render no longer computes rects inline.
- Each overlay gets `fn rect(area, &state) -> Rect` sized to content (§6). B83 hit map is built
  from these same rects.

## 4. Context-aware hints (U3, B80 leftover)

`fn hints(&self) -> (Vec<&str>, tail)` keyed on top overlay, else on (tab, focused field). Rendered
through existing `fit_hints`. Examples: confirm → `y:Delete n/Esc:Cancel`; picker → picker keys;
text editing → `Enter:Apply Esc:Revert`; Operate on Execute → `Enter:Run`.

## 5. Focus and key map (U1, U2, U9) — breaking

Proposed (each item can be accepted/rejected individually):

| # | Today | Proposed |
|---|---|---|
| K1 | Focus layers NavBar → TopField → Content; Esc cycles them | Drop the NavBar layer. `1/2/3` switch tabs; `Tab` / `Shift+Tab` next/previous tab from anywhere (no overlay open). Field-group `Tab` cycling is removed; `↑↓` move fields, `←→` cycle radio values. Esc: close overlay → cancel edit → move focus to the tab's first field; never wraps |
| K2 | `e` = edit (Config) / **execute from any field** (Operate) / jump (View) | `e` = edit everywhere. Run operation = `Enter` on Execute or `Ctrl+R` from any Operate field |
| K3 | `d` = delete (Config) / dry-run toggle (Operate) | Config keeps `d` (+`y` confirm). Operate dry-run → `D`; serial `s` → `S` (rare toggles, uppercase avoids accidents) |
| K4 | Broad run/exec starts immediately | Confirm overlay when target = All and dry-run off: `Run exec on N hosts? [y] Run [n/Esc] Cancel` |
| K5 | — | `y` yank: copies the focused value / selected result row via OSC 52 (ships before B83 mouse capture removes drag-select) |

Help text (B81) and `tui.md` are regenerated from the final table.

## 6. Content-aware popups (U4)

All popups use `fixed_centered_rect` (B79) with size = content + chrome, clamped to the screen;
percent sizing removed. Picker width = longest item/title + 6; title never truncated below 40 cols.

## 7. Chrome (U7, U8)

- Tab bar: 1 row (`1:Config 2:Operate 3:View … sshi v1.7.0`), no box. Saves 2 rows.
- Status: 1 row (hints), errors replace hints until Esc. Saves 1 row.
- Breadcrumb: `~` for `$HOME`, middle ellipsis when too long.

## 8. Value rendering (U5, U6)

- Checkout status for never-checked hosts: `? unknown` (dim), not `✗ offline`; last-seen `—`.
- Empty optional Config values render `(none)` (dim) consistently.

## 9. Delivery

Separate commits, each green and verified on the real binary at 80×24:
1. Overlay stack + overlays render over body (§2).
2. Shared geometry (§3) + content-aware popups (§6).
3. Context hints (§4).
4. Chrome + breadcrumb + value rendering (§7, §8).
5. Key map K1–K5 (§5) + help text + `tui.md`.

Tests: `overlays()` order table test; one test per overlay that a key is consumed by the top
overlay only; render tests assert the body is still drawn under a confirm; key-map tests per K-item.

## 10. Open questions for approval

- Q1: accept K1–K5 as written? (especially K1 removing NavBar focus, K2 `Ctrl+R`)
- Q2: resolved (user, 2026-10-21) — `Tab` / `Shift+Tab` switch tabs; in-group `Tab` cycling removed.

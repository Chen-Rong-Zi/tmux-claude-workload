# Scrolloff=7 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `scrolloff` of 7 rows to Tree View navigation so the selected item is never closer than 7 rows to the top or bottom edge of the viewport.

**Architecture:** Add a `scrolloff` field to `App`, modify `ensure_visible()` and the draw-loop clamping logic to enforce the margin. Only affects Tree View.

**Tech Stack:** Rust, ratatui

---

### Task 1: Add scrolloff and enforce it in navigation + draw

**Files:**
- Modify: `src/app.rs`

Three changes: (1) new field + init, (2) `ensure_visible()` for keyboard navigation, (3) draw-loop clamping for auto-follow.

- [ ] **Step 1: Add `scrolloff` field to `App` struct**

After `git_debounce_active` (around line 186), add:

```rust
    /// Minimum visible rows above/below the selected item (scrolloff).
    scrolloff: usize,
```

- [ ] **Step 2: Initialize in `App::new()`**

After `git_debounce_active: false,` (around line 219), add:

```rust
            scrolloff: 7,
```

- [ ] **Step 3: Update `ensure_visible()` to respect scrolloff**

Replace the current `ensure_visible` method (lines 540-546):

```rust
    fn ensure_visible(&mut self) {
        let visual = tree::item_to_visual_row(&self.tree_items, self.selected, self.last_width);
        // Ensure at least scrolloff rows above the selected item
        if visual < self.scroll_offset + self.scrolloff {
            self.scroll_offset = visual.saturating_sub(self.scrolloff);
        }
        // Upper bound adjusted during rendering
    }
```

- [ ] **Step 4: Update draw-loop clamping (Tree View only) to respect scrolloff**

In the main draw closure, Tree View branch (around lines 1223-1231), replace the 5-line clamping block:

**Before:**
```rust
                        // Default: just ensure selected is visible
                        let selected_visual =
                            tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
                        if selected_visual >= app.scroll_offset + visible_height {
                            app.scroll_offset = selected_visual.saturating_sub(visible_height - 1);
                        }
                        if selected_visual < app.scroll_offset {
                            app.scroll_offset = selected_visual;
                        }
```

**After:**
```rust
                        // Default: enforce scrolloff margin
                        let selected_visual =
                            tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
                        let soff = app.scrolloff;
                        // Bottom margin: selected must be at most (visible_height - soff - 1) from top
                        if selected_visual + soff >= app.scroll_offset + visible_height {
                            let target = selected_visual.saturating_sub(visible_height.saturating_sub(soff + 1));
                            app.scroll_offset = target;
                        }
                        // Top margin: selected must be at least soff from top
                        if selected_visual < app.scroll_offset + soff {
                            app.scroll_offset = selected_visual.saturating_sub(soff);
                        }
```

- [ ] **Step 5: Update stage-2 draw clamping (same logic, duplicate)**

There's a second copy of the same clamping code in the Stage 2 draw (around lines 1003-1010). Apply the same replacement:

**Before (lines 1003-1010):**
```rust
        let selected_visual =
            tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
        if selected_visual >= app.scroll_offset + visible_height {
            app.scroll_offset = selected_visual.saturating_sub(visible_height - 1);
        }
        if selected_visual < app.scroll_offset {
            app.scroll_offset = selected_visual;
        }
```

**After:**
```rust
        let selected_visual =
            tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
        let soff = app.scrolloff;
        if selected_visual + soff >= app.scroll_offset + visible_height {
            let target = selected_visual.saturating_sub(visible_height.saturating_sub(soff + 1));
            app.scroll_offset = target;
        }
        if selected_visual < app.scroll_offset + soff {
            app.scroll_offset = selected_visual.saturating_sub(soff);
        }
```

- [ ] **Step 6: Build and verify**

Run: `cargo build`
Expected: compiles with no errors or warnings

- [ ] **Step 7: Run tests**

Run: `cargo test`
Expected: 206 tests pass

- [ ] **Step 8: Format and commit**

```bash
cargo fmt
git add src/app.rs
git commit -m "feat: add scrolloff=7 for tree view navigation"
```

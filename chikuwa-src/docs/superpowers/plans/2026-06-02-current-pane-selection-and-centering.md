# Current Pane Selection and Centered Scrolling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select the pane where chikuwa is running on startup (using TMUX_PANE), and center the selected row when user navigates.

**Architecture:** Add a `first_selection` flag to track initial startup, use `TMUX_PANE` env var to find the correct pane, and modify `ensure_visible()` to center the selected visual row instead of just making it visible.

**Tech Stack:** Rust, ratatui, tmux

---

## File Structure

- **Modify:** `src/app.rs` — Add flag, add pane finder, modify selection and scroll logic
- **Modify:** `src/ui/tree.rs` — Add helper function to find pane by ID

---

### Task 1: Add `first_selection` flag to App struct

**Files:**
- Modify: `src/app.rs:142-169` (App struct definition)
- Modify: `src/app.rs:171-197` (App::new())

- [ ] **Step 1: Add the field to App struct**

Add after `user_navigated` field:

```rust
    /// True when the user has navigated manually (Up/Down/Top/Bottom).
    /// Prevents auto-follow of the active tmux pane until the user selects an item.
    user_navigated: bool,
    /// True until the first selection is made (used to select TMUX_PANE on startup).
    first_selection: bool,
```

- [ ] **Step 2: Initialize the field in App::new()**

Add after `user_navigated: false,`:

```rust
            user_navigated: false,
            first_selection: true,
```

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: add first_selection flag to App struct"
```

---

### Task 2: Add helper function to find pane by ID in tree

**Files:**
- Modify: `src/ui/tree.rs:425-462` (after find_active_index)

- [ ] **Step 1: Add find_pane_index function**

Add after `find_active_index` function:

```rust
/// Find the index of a pane by its pane_id (e.g., "%0").
pub fn find_pane_index(items: &[TreeItem], pane_id: &str) -> Option<usize> {
    items.iter().position(|item| {
        matches!(
            item,
            TreeItem::Pane { pane: p, .. } if p.pane_id == pane_id
        )
    })
}
```

- [ ] **Step 2: Write test for find_pane_index**

Add to the test module in `src/ui/tree.rs`:

```rust
    #[test]
    fn test_find_pane_index() {
        let raw = "main\t1\t0\tzsh\t1\t%0\t0\tbash\t1\t/home\t\n\
                    main\t1\t0\tzsh\t0\t%1\t1\tvim\t0\t/tmp\t\n";
        let sessions = build_tree(raw, &HashMap::new());
        let items = flatten(&sessions, &HashSet::new());

        // Should find %0 (first pane)
        let idx0 = find_pane_index(&items, "%0");
        assert!(idx0.is_some());

        // Should find %1 (second pane)
        let idx1 = find_pane_index(&items, "%1");
        assert!(idx1.is_some());

        // Should not find non-existent pane
        let idx2 = find_pane_index(&items, "%99");
        assert!(idx2.is_none());
    }
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test test_find_pane_index 2>&1`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/ui/tree.rs
git commit -m "feat: add find_pane_index helper function"
```

---

### Task 3: Modify rebuild_tree to use TMUX_PANE on first selection

**Files:**
- Modify: `src/app.rs:305-324` (rebuild_tree function)

- [ ] **Step 1: Update rebuild_tree to use TMUX_PANE on first call**

Replace the `rebuild_tree` function:

```rust
    fn rebuild_tree(&mut self) {
        self.tree_items = tree::flatten(&self.sessions, &self.collapsed);

        // On first selection, try to select the pane where chikuwa is running
        if self.first_selection {
            if let Ok(pane_id) = std::env::var("TMUX_PANE") {
                if let Some(idx) = tree::find_pane_index(&self.tree_items, &pane_id) {
                    self.selected = idx;
                    self.first_selection = false;
                }
            }
            // Fall back to active pane if TMUX_PANE not found
            if self.first_selection && !self.user_navigated {
                if let Some(active_idx) = tree::find_active_index(&self.sessions, &self.tree_items) {
                    self.selected = active_idx;
                }
            }
            self.first_selection = false;
        } else if !self.user_navigated {
            // Follow the active (focused) pane/window when user hasn't navigated manually
            if let Some(active_idx) = tree::find_active_index(&self.sessions, &self.tree_items) {
                self.selected = active_idx;
            }
        }

        // Clamp selected index
        if !self.tree_items.is_empty() && self.selected >= self.tree_items.len() {
            self.selected = self.tree_items.len() - 1;
        }
        // Ensure selected is not a Session item
        self.snap_to_selectable();
        // Clamp scroll offset to valid visual row range
        let total_visual = tree::total_visual_rows(&self.tree_items, self.last_width);
        if total_visual > 0 && self.scroll_offset >= total_visual {
            self.scroll_offset = total_visual - 1;
        }
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/app.rs
git commit -m "feat: select TMUX_PANE pane on first startup"
```

---

### Task 4: Add center_selection function and modify ensure_visible

**Files:**
- Modify: `src/app.rs:402-408` (ensure_visible function)

- [ ] **Step 1: Add center_selection function after ensure_visible**

Add after `ensure_visible` function:

```rust
    fn ensure_visible(&mut self) {
        let visual = tree::item_to_visual_row(&self.tree_items, self.selected, self.last_width);
        if visual < self.scroll_offset {
            self.scroll_offset = visual;
        }
        // Upper bound adjusted during rendering
    }

    /// Center the selected item in the viewport.
    fn center_selection(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        let visual = tree::item_to_visual_row(&self.tree_items, self.selected, self.last_width);
        let total_visual = tree::total_visual_rows(&self.tree_items, self.last_width);

        // Calculate center position
        let half_height = visible_height / 2;
        let desired_offset = visual.saturating_sub(half_height);

        // Clamp to valid range
        let max_offset = total_visual.saturating_sub(visible_height);
        self.scroll_offset = desired_offset.min(max_offset);
    }
```

- [ ] **Step 2: Update move_up to use center_selection**

Replace `move_up` function:

```rust
    fn move_up(&mut self) {
        self.user_navigated = true;
        let mut idx = self.selected;
        while idx > 0 {
            idx -= 1;
            if self.tree_items[idx].is_selectable() {
                self.selected = idx;
                self.ensure_visible();
                return;
            }
        }
    }
```

(Note: center_selection requires visible_height which isn't available in move_up. We'll handle centering in the render loop instead.)

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: add center_selection function"
```

---

### Task 5: Center selected row during render after navigation

**Files:**
- Modify: `src/app.rs:142-169` (App struct - add pending_center field)
- Modify: `src/app.rs:338-362` (move_up/move_down - set pending_center)
- Modify: `src/app.rs:364-382` (move_top/move_bottom - set pending_center)
- Modify: `src/app.rs:862-873` (render loop - apply centering)

- [ ] **Step 1: Add pending_center flag to App struct**

Add after `first_selection` field:

```rust
    /// True until the first selection is made (used to select TMUX_PANE on startup).
    first_selection: bool,
    /// When true, center the selected row on next render.
    pending_center: bool,
```

- [ ] **Step 2: Initialize pending_center in App::new()**

Add after `first_selection: true,`:

```rust
            first_selection: true,
            pending_center: false,
```

- [ ] **Step 3: Set pending_center in move_up**

Replace `move_up` function:

```rust
    fn move_up(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        let mut idx = self.selected;
        while idx > 0 {
            idx -= 1;
            if self.tree_items[idx].is_selectable() {
                self.selected = idx;
                self.ensure_visible();
                return;
            }
        }
    }
```

- [ ] **Step 4: Set pending_center in move_down**

Replace `move_down` function:

```rust
    fn move_down(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        let mut idx = self.selected;
        while idx < self.tree_items.len().saturating_sub(1) {
            idx += 1;
            if self.tree_items[idx].is_selectable() {
                self.selected = idx;
                self.ensure_visible();
                return;
            }
        }
    }
```

- [ ] **Step 5: Set pending_center in move_top and move_bottom**

Replace `move_top` function:

```rust
    fn move_top(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        if let Some(idx) = self.tree_items.iter().position(|item| item.is_selectable()) {
            self.selected = idx;
        }
    }
```

Replace `move_bottom` function:

```rust
    fn move_bottom(&mut self) {
        self.user_navigated = true;
        self.pending_center = true;
        if let Some(idx) = self
            .tree_items
            .iter()
            .rposition(|item| item.is_selectable())
        {
            self.selected = idx;
        }
    }
```

- [ ] **Step 6: Apply centering in render loop**

Replace the scroll adjustment section in the render loop:

```rust
            // Adjust scroll for visible area (visual rows, no outer border)
            app.tree_area = chunks[1];
            let visible_height = chunks[1].height as usize;
            app.last_width = chunks[1].width;

            if app.pending_center {
                app.center_selection(visible_height);
                app.pending_center = false;
            } else {
                // Default: just ensure selected is visible
                let selected_visual =
                    tree::item_to_visual_row(&app.tree_items, app.selected, app.last_width);
                if selected_visual >= app.scroll_offset + visible_height {
                    app.scroll_offset = selected_visual.saturating_sub(visible_height - 1);
                }
                if selected_visual < app.scroll_offset {
                    app.scroll_offset = selected_visual;
                }
            }
```

- [ ] **Step 7: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/app.rs
git commit -m "feat: center selected row on navigation"
```

---

### Task 6: Run full pre-commit checks and final commit

- [ ] **Step 1: Run fmt and clippy**

Run: `cargo fmt && cargo clippy -- -D warnings 2>&1`
Expected: No warnings

- [ ] **Step 2: Run all tests**

Run: `cargo test 2>&1`
Expected: All 124+ tests pass

- [ ] **Step 3: Squash commits if needed (optional)**

If multiple intermediate commits were made, consider squashing:

```bash
git rebase -i HEAD~5
# Mark intermediate commits as 'fixup' or 'squash'
```

---

## Summary

This plan implements:

1. **TMUX_PANE selection on startup** — Uses the environment variable to find the pane where chikuwa is running, solving the multi-client issue
2. **Centered scrolling on navigation** — When user presses j/k/g/G or arrow keys, the selected row is centered in the viewport
3. **Non-navigation keeps scroll position** — When agent states update or tree content changes, scroll position is adjusted minimally to keep selected visible but not re-centered

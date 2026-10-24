# Office View → Pixtuoid Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the existing Office View (agent room renderer) with a launcher that suspends the TUI, runs `pixtuoid run` as a foreground process, then resumes the TUI when it exits.

**Architecture:** Strip all ~800 lines of Office View rendering code and all `office::*` references. Repurpose `ViewMode::Office` as a trigger: on toggle to Office, chikuwa exits alternate screen, runs `pixtuoid run`, then re-enters alternate screen and goes back to Tree View. The `h`/`l` keys cycle Tree → pixtuoid → Tree.

**Tech Stack:** Rust, tokio, crossterm

---

### Task 1: Delete office.rs and clean up module registration

**Files:**
- Delete: `src/ui/office.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Remove `pub mod office;` from `src/ui/mod.rs`**

Replace the file contents to only have:

```rust
pub mod status_bar;
pub mod theme;
pub mod tree;
```

- [ ] **Step 2: Delete the office.rs file**

Run: `rm src/ui/office.rs`

- [ ] **Step 3: Build to check for compilation errors**

Run: `cargo build 2>&1 | head -20`
Expected: Errors about missing `office` module from `app.rs` imports

- [ ] **Step 4: Commit**

```bash
git add src/ui/mod.rs
git rm src/ui/office.rs
git commit -m "refactor: remove office view module"
```

---

### Task 2: Remove all office::* references from app.rs

**Files:**
- Modify: `src/app.rs`

Remove all imports, method calls, and branches that reference the old office module.

- [ ] **Step 1: Remove `office` from the import**

Find and change:
```rust
use crate::ui::{office, status_bar, theme, tree};
```
to:
```rust
use crate::ui::{status_bar, theme, tree};
```

- [ ] **Step 2: Remove Office branch in `rebuild_tree()` (line 388-390)**

Delete these lines:
```rust
        // Skip auto-follow in office view — selected is an agent index, not a tree index
        if self.view_mode == ViewMode::Office {
            return;
        }
```

- [ ] **Step 3: Remove Office branches in `move_up()` / `move_down()` / `move_top()` / `move_bottom()`**

For each of these methods, remove the `ViewMode::Office` arm. After removal, each match becomes a single-arm match; simplify to just the `ViewMode::Tree` body directly.

Specifically:

In `move_up()` (~line 443), remove the `ViewMode::Office => { if self.selected > 0 { self.selected -= 1; } }` arm.

In `move_down()` (~line 455), remove the entire Office arm (the `ViewMode::Office => { ... }` block).

In `move_top()` (~line 478), remove the Office arm.

In `move_bottom()` (~line 497), remove the Office arm.

After each removal, simplify the single-arm `match` to just the contained code (keep only the tree logic).

- [ ] **Step 4: Remove Office branch in `handle_select()` (lines 715-727)**

Delete the entire `if self.view_mode == ViewMode::Office { ... }` block at the start of `handle_select()`.

- [ ] **Step 5: Remove Office text variants from title bar**

In `render_title()`, find the two occurrences of `ViewMode::Office` in the view_mode match:

In the running-animation branch (~line 916):
```rust
ViewMode::Office => ":office  ",
```
Change to:
```rust
ViewMode::Office => ":pixtuoid  ",
```

In the static branch (~line 924):
```rust
ViewMode::Office => "  chikuwa:office  ",
```
Change to:
```rust
ViewMode::Office => "  chikuwa:pixtuoid  ",
```

- [ ] **Step 6: Remove Office rendering and navigation from draw loop**

In the main draw closure, find the `ViewMode::Office => { ... }` branch (around line 1255-1295). Replace the entire branch with:

```rust
                ViewMode::Office => {
                    // Pixtuoid mode — no TUI rendering; handled in ToggleView action
                }
```

- [ ] **Step 7: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: compiles successfully (with possible dead_code warnings on `build_subagent_data` and related subagent methods)

- [ ] **Step 8: Commit**

```bash
git add src/app.rs
git commit -m "refactor: remove all office view references from app.rs"
```

---

### Task 3: Implement pixtuoid runner in ToggleView action

**Files:**
- Modify: `src/app.rs`

The `ToggleView` action currently just flips the view_mode enum. When the user toggles **to** Office (pixtuoid), we need to:
1. Exit the TUI (disable raw mode, leave alternate screen)
2. Run `pixtuoid run` as a foreground subprocess
3. Re-enter the TUI
4. Set view_mode back to Tree

- [ ] **Step 1: Remove `ToggleView` from `apply_key_action()`**

The current `apply_key_action()` handles `ToggleView` as a simple enum flip (lines ~819-825). Delete that arm since we'll handle it in the event loop instead:

```rust
        Action::ToggleView => {
            app.view_mode = match app.view_mode {
                ViewMode::Tree => ViewMode::Office,
                ViewMode::Office => ViewMode::Tree,
            };
            app.selected = 0;
            app.scroll_offset = 0;
            app.user_navigated = true;
        }
```

Also remove `ToggleView` from the `Action::Select | Action::None => {}` catch-all arm (it should now match the default case).

- [ ] **Step 2: Add `ToggleView` handling in the event loop**

In the main event loop, the key handling code (around line 1296) currently looks like:

```rust
                if action == Action::Select {
                    app.handle_select().await?;
                } else if apply_key_action(&mut app, action) {
                    break;
                }
```

Replace it with:

```rust
                if action == Action::Select {
                    app.handle_select().await?;
                } else if action == Action::ToggleView {
                    if app.view_mode == ViewMode::Tree {
                        // Exit TUI
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;

                        // Run pixtuoid in foreground
                        let status = tokio::process::Command::new("pixtuoid")
                            .arg("run")
                            .status()
                            .await;

                        // Re-enter TUI
                        enable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            EnableMouseCapture
                        )?;
                        terminal.hide_cursor()?;

                        if let Err(e) = status {
                            eprintln!("Warning: pixtuoid failed: {}", e);
                        }

                        // Reset to Tree view and refresh
                        app.view_mode = ViewMode::Tree;
                        app.refresh().await?;
                    }
                } else if apply_key_action(&mut app, action) {
                    break;
                }
```

This intercepts `ToggleView` before `apply_key_action()`, runs pixtuoid when in Tree mode, and resets view_mode back to Tree on return.

Note: The `?` on `disable_raw_mode()` and `enable_raw_mode()` is OK because these only fail on terminal issues that would crash chikuwa anyway.

Also add the required crossterm imports at the top of the file if not already present (they should be since `run()` uses them):

```rust
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
```

- [ ] **Step 3: Build to verify**

Run: `cargo build 2>&1`
Expected: compiles successfully

- [ ] **Step 4: Run tests**

Run: `cargo test 2>&1`
Expected: all tests pass (note: the `quit_action_requests_main_loop_exit` test should still pass since `ToggleView` is still a valid action, just handled differently)

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/app.rs
git commit -m "feat: replace office view with pixtuoid runner"
```

---

### Task 4: Clean up unused subagent data infrastructure

**Files:**
- Check: `src/app.rs`

After removing office view, `build_subagent_data()` may only be used in the Tree View (via `subagent_data`). Verify it's still used. If the only remaining callers are in Tree View rendering, leave as-is. If truly dead code, remove it.

- [ ] **Step 1: Check for remaining `build_subagent_data` callers**

Run: `grep -n "build_subagent_data\|subagent_data" src/app.rs | grep -v "test\|#\["`

Expected output should show at least one remaining use in the draw loop's Tree View branch. If so, no further cleanup needed — `build_subagent_data` is still used by tree rendering.

- [ ] **Step 2: Remove `#[allow(dead_code)]` from unused subagent methods if needed**

If `get_subagents_for_pane()` and `get_completed_count()` now have dead_code warnings, prefix them with `#[allow(dead_code)]` or remove them if they're truly unused. Leave subagent fields (`subagent_states`, `completed_subagent_counts`) and `merge_subagent_state()` in place — they're still used by IPC events.

- [ ] **Step 3: Final build and test**

```bash
cargo build
cargo test
```

Expected: clean build, all tests pass

- [ ] **Step 4: Commit if any changes**

```bash
git add src/app.rs
git commit -m "chore: clean up unused code after office view removal"
```

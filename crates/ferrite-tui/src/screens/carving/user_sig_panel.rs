//! Overlay panel for browsing, adding, editing, and deleting user-defined
//! carving signatures (opened with `u` from the Carving screen).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{CarvingState, FormMode, UserSigForm};
use super::helpers::fmt_bytes;

// ── Geometry helper ───────────────────────────────────────────────────────────

/// Returns a [`Rect`] centred within `r` sized to `(pct_w × pct_h)` percent.
fn centered_rect(pct_w: u16, pct_h: u16, r: Rect) -> Rect {
    let vpad = (100 - pct_h) / 2;
    let hpad = (100 - pct_w) / 2;
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(vpad),
            Constraint::Percentage(pct_h),
            Constraint::Percentage(vpad),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(hpad),
            Constraint::Percentage(pct_w),
            Constraint::Percentage(hpad),
        ])
        .split(vert[1])[1]
}

// ── Render ────────────────────────────────────────────────────────────────────

impl CarvingState {
    /// Draw the user-signatures list overlay.  Called last in `render()` so it
    /// appears on top of all other widgets.
    pub(super) fn render_user_panel(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(70, 65, area);
        frame.render_widget(Clear, popup);

        let title = " Custom Signatures ";
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        // Split: list body + footer line(s).
        let footer_lines: u16 = if self.editing_import { 2 } else { 1 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(footer_lines)])
            .split(inner);

        // ── List body ─────────────────────────────────────────────────────────
        if self.user_sigs.is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No custom signatures.  Press a to add one.",
                    Style::default().fg(Color::DarkGray),
                ))),
                rows[0],
            );
        } else {
            let items: Vec<ListItem> = self
                .user_sigs
                .iter()
                .enumerate()
                .map(|(i, def)| {
                    let sel = i == self.user_panel_sel;
                    let prefix = if sel { "▶ " } else { "  " };
                    let size_str = fmt_bytes(def.max_size);
                    let label = format!(
                        "{prefix}{:<24} .{:<8} {}  (max {})",
                        def.name, def.extension, def.header, size_str
                    );
                    let style = if sel {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(label).style(style)
                })
                .collect();

            let mut list_state = ListState::default();
            list_state.select(Some(self.user_panel_sel));
            frame.render_stateful_widget(
                List::new(items),
                rows[0],
                &mut list_state,
            );
        }

        // ── Footer ────────────────────────────────────────────────────────────
        if self.editing_import {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Import path: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{}\u{2588}", self.user_import_path),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])),
                rows[1],
            );
        } else if self.user_confirm_delete {
            let name = self
                .user_sigs
                .get(self.user_panel_sel)
                .map(|d| d.name.as_str())
                .unwrap_or("?");
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" Delete '{name}'? "),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("y ", Style::default().fg(Color::Red)),
                    Span::styled("/ ", Style::default().fg(Color::DarkGray)),
                    Span::styled("n ", Style::default().fg(Color::Green)),
                ])),
                rows[1],
            );
        } else {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " a:add  e:edit  d:delete  i:import  Esc:close",
                    Style::default().fg(Color::DarkGray),
                ))),
                rows[1],
            );
        }
    }

    /// Draw the add/edit form dialog on top of the user-signatures panel.
    pub(super) fn render_user_form(&self, frame: &mut Frame, area: Rect) {
        let Some(form) = &self.user_sig_form else {
            return;
        };

        let popup = centered_rect(60, 55, area);
        frame.render_widget(Clear, popup);

        let title = match form.mode {
            FormMode::Add => " Add Signature ",
            FormMode::Edit(_) => " Edit Signature ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        // Layout: 5 field rows + blank + error + blank + help
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Name
                Constraint::Length(1), // Extension
                Constraint::Length(1), // Header
                Constraint::Length(1), // Footer
                Constraint::Length(1), // Max size
                Constraint::Length(1), // blank
                Constraint::Length(1), // error / blank
                Constraint::Length(1), // blank
                Constraint::Length(1), // help
            ])
            .split(inner);

        let fields = [
            ("Name      ", &form.name),
            ("Extension ", &form.extension),
            ("Header    ", &form.header),
            ("Footer    ", &form.footer),
            ("Max size  ", &form.max_size_str),
        ];

        for (i, (label, value)) in fields.iter().enumerate() {
            let active = form.field == i;
            let label_style = Style::default().fg(Color::DarkGray);
            let value_str = if active {
                format!("{value}\u{2588}")
            } else {
                value.to_string()
            };
            let value_style = if active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {label}: "), label_style),
                    Span::styled(value_str, value_style),
                ])),
                rows[i],
            );
        }

        // Error line
        if let Some(err) = &form.error {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" ⚠ {err}"),
                    Style::default().fg(Color::Red),
                ))),
                rows[6],
            );
        }

        // Help line
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " Tab: next field   Ctrl+S / Enter on last: save   Esc: cancel",
                Style::default().fg(Color::DarkGray),
            ))),
            rows[8],
        );
    }
}

// ── Form key handler ──────────────────────────────────────────────────────────

/// Outcome of processing one key event against an open [`UserSigForm`].
#[derive(Debug, PartialEq)]
pub(super) enum FormAction {
    /// Continue editing — put the (possibly modified) form back.
    None,
    /// User confirmed: validate and save.
    Submit,
    /// User pressed Esc: discard changes.
    Cancel,
}

/// Process a single key event for the form.  Returns the action to take.
pub(super) fn handle_form_key(
    form: &mut UserSigForm,
    code: KeyCode,
    mods: KeyModifiers,
) -> FormAction {
    let can_type = mods.is_empty() || mods == KeyModifiers::SHIFT;

    // Clear stale error on any key except pure modifier presses.
    form.error = None;

    match code {
        KeyCode::Esc => return FormAction::Cancel,

        // Ctrl+S = save from any field.
        KeyCode::Char('s') if mods.contains(KeyModifiers::CONTROL) => {
            return FormAction::Submit;
        }

        // Enter on the last field (max_size) = save; otherwise advance.
        KeyCode::Enter => {
            if form.field == 4 {
                return FormAction::Submit;
            }
            form.field += 1;
        }

        KeyCode::Tab => {
            form.field = (form.field + 1) % 5;
        }
        KeyCode::BackTab => {
            form.field = form.field.saturating_sub(1);
        }

        KeyCode::Backspace => match form.field {
            0 => {
                form.name.pop();
            }
            1 => {
                form.extension.pop();
            }
            2 => {
                form.header.pop();
            }
            3 => {
                form.footer.pop();
            }
            _ => {
                form.max_size_str.pop();
            }
        },

        KeyCode::Char(c) if can_type => match form.field {
            0 => form.name.push(c),
            1 => form.extension.push(c),
            2 => form.header.push(c),
            3 => form.footer.push(c),
            _ => {
                if c.is_ascii_digit() {
                    form.max_size_str.push(c);
                }
            }
        },

        _ => {}
    }

    FormAction::None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::carving::UserSigForm;

    fn make_form() -> UserSigForm {
        UserSigForm {
            mode: FormMode::Add,
            field: 0,
            name: String::new(),
            extension: String::new(),
            header: String::new(),
            footer: String::new(),
            max_size_str: String::new(),
            error: None,
        }
    }

    #[test]
    fn tab_advances_field() {
        let mut form = make_form();
        assert_eq!(form.field, 0);
        handle_form_key(&mut form, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(form.field, 1);
        handle_form_key(&mut form, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(form.field, 2);
    }

    #[test]
    fn tab_wraps_at_end() {
        let mut form = make_form();
        form.field = 4;
        handle_form_key(&mut form, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(form.field, 0);
    }

    #[test]
    fn back_tab_goes_back() {
        let mut form = make_form();
        form.field = 2;
        handle_form_key(&mut form, KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(form.field, 1);
    }

    #[test]
    fn back_tab_does_not_underflow() {
        let mut form = make_form();
        form.field = 0;
        handle_form_key(&mut form, KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(form.field, 0);
    }

    #[test]
    fn enter_on_last_field_submits() {
        let mut form = make_form();
        form.field = 4;
        assert_eq!(
            handle_form_key(&mut form, KeyCode::Enter, KeyModifiers::NONE),
            FormAction::Submit
        );
    }

    #[test]
    fn enter_on_non_last_field_advances() {
        let mut form = make_form();
        form.field = 2;
        assert_eq!(
            handle_form_key(&mut form, KeyCode::Enter, KeyModifiers::NONE),
            FormAction::None
        );
        assert_eq!(form.field, 3);
    }

    #[test]
    fn esc_cancels() {
        let mut form = make_form();
        assert_eq!(
            handle_form_key(&mut form, KeyCode::Esc, KeyModifiers::NONE),
            FormAction::Cancel
        );
    }

    #[test]
    fn ctrl_s_submits_from_any_field() {
        let mut form = make_form();
        form.field = 1;
        assert_eq!(
            handle_form_key(&mut form, KeyCode::Char('s'), KeyModifiers::CONTROL),
            FormAction::Submit
        );
    }

    #[test]
    fn char_appends_to_current_field() {
        let mut form = make_form();
        form.field = 0;
        handle_form_key(&mut form, KeyCode::Char('H'), KeyModifiers::SHIFT);
        handle_form_key(&mut form, KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(form.name, "Hi");
    }

    #[test]
    fn max_size_field_only_accepts_digits() {
        let mut form = make_form();
        form.field = 4;
        handle_form_key(&mut form, KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(form.max_size_str, ""); // letters rejected
        handle_form_key(&mut form, KeyCode::Char('1'), KeyModifiers::NONE);
        handle_form_key(&mut form, KeyCode::Char('0'), KeyModifiers::NONE);
        assert_eq!(form.max_size_str, "10");
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut form = make_form();
        form.field = 2;
        form.header = "AA BB".to_string();
        handle_form_key(&mut form, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(form.header, "AA B");
    }

    #[test]
    fn any_key_clears_error() {
        let mut form = make_form();
        form.error = Some("bad input".to_string());
        handle_form_key(&mut form, KeyCode::Tab, KeyModifiers::NONE);
        assert!(form.error.is_none());
    }
}

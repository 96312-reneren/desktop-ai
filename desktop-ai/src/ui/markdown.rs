use egui::{Color32, Label, RichText, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

struct Run {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

struct State {
    font_size: f32,
    bold: bool,
    italic: bool,
    strike: bool,
    runs: Vec<Run>,
    cur: String,

    code_block_buf: Option<String>,

    link_active: bool,
    link_buf: String,
    link_url: String,

    list_depth: u32,
    blockquote_depth: u32,
    font_size_stack: Vec<f32>,

    table_rows: Vec<Vec<String>>,
    table_is_head: bool,
    table_cur_row: Vec<String>,
    in_table_cell: bool,
    table_cell_buf: String,
    table_counter: u32,
}

impl State {
    fn new(font_size: f32) -> Self {
        Self {
            font_size,
            bold: false,
            italic: false,
            strike: false,
            runs: Vec::new(),
            cur: String::new(),
            code_block_buf: None,
            link_active: false,
            link_buf: String::new(),
            link_url: String::new(),
            list_depth: 0,
            blockquote_depth: 0,
            font_size_stack: Vec::new(),
            table_rows: Vec::new(),
            table_is_head: false,
            table_cur_row: Vec::new(),
            in_table_cell: false,
            table_cell_buf: String::new(),
            table_counter: 0,
        }
    }

    fn push_run_text(&mut self, text: String, code: bool) {
        if text.is_empty() {
            return;
        }
        self.runs.push(Run {
            text,
            bold: self.bold,
            italic: self.italic,
            strike: self.strike,
            code,
        });
    }

    fn close_cur_run(&mut self) {
        if !self.cur.is_empty() {
            let text = std::mem::take(&mut self.cur);
            self.push_run_text(text, false);
        }
    }

    fn flush(&mut self, ui: &mut Ui) {
        self.close_cur_run();
        if self.runs.is_empty() {
            return;
        }
        let indent = self.blockquote_depth as f32 * 12.0;
        let fs = self.font_size;
        ui.horizontal_wrapped(|ui| {
            if indent > 0.0 {
                ui.add_space(indent);
            }
            for run in self.runs.drain(..) {
                let mut rt = RichText::new(&run.text).size(fs);
                if run.bold {
                    rt = rt.strong();
                }
                if run.italic {
                    rt = rt.italics();
                }
                if run.strike {
                    rt = rt.strikethrough();
                }
                if run.code {
                    rt = rt
                        .monospace()
                        .color(Color32::from_rgb(220, 200, 100))
                        .background_color(Color32::from_rgb(50, 50, 50));
                }
                ui.add(Label::new(rt).selectable(true));
            }
        });
    }

    fn push_text(&mut self, t: &str) {
        if let Some(buf) = self.code_block_buf.as_mut() {
            buf.push_str(t);
        } else if self.link_active {
            self.link_buf.push_str(t);
        } else if self.in_table_cell {
            self.table_cell_buf.push_str(t);
        } else {
            self.cur.push_str(t);
        }
    }

    fn start(&mut self, ui: &mut Ui, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush(ui);
                self.font_size_stack.push(self.font_size);
                let bump = match level {
                    pulldown_cmark::HeadingLevel::H1 => 4.0,
                    pulldown_cmark::HeadingLevel::H2 => 2.5,
                    pulldown_cmark::HeadingLevel::H3 => 1.5,
                    _ => 0.8,
                };
                self.font_size += bump;
            }
            Tag::CodeBlock(_) => {
                self.flush(ui);
                self.code_block_buf = Some(String::new());
            }
            Tag::Emphasis => {
                self.close_cur_run();
                self.italic = true;
            }
            Tag::Strong => {
                self.close_cur_run();
                self.bold = true;
            }
            Tag::Strikethrough => {
                self.close_cur_run();
                self.strike = true;
            }
            Tag::BlockQuote(_) => {
                self.flush(ui);
                self.blockquote_depth += 1;
            }
            Tag::List(_) => {
                self.flush(ui);
                self.list_depth += 1;
            }
            Tag::Item => {
                self.flush(ui);
                ui.add_space(2.0);
                let indent = self.list_depth.saturating_sub(1) as f32 * 16.0;
                if indent > 0.0 {
                    ui.add_space(indent);
                }
                ui.label(RichText::new("• ").size(self.font_size).strong());
            }
            Tag::Link { dest_url, .. } => {
                self.flush(ui);
                self.link_active = true;
                self.link_url = dest_url.into_string();
                self.link_buf.clear();
            }
            Tag::Image { dest_url, .. } => {
                self.push_text(&format!(" [图片: {}] ", dest_url));
            }
            Tag::Table { .. } => {
                self.flush(ui);
                self.table_rows.clear();
            }
            Tag::TableHead => {
                self.table_is_head = true;
                self.table_cur_row.clear();
            }
            Tag::TableRow => {
                self.table_cur_row.clear();
            }
            Tag::TableCell => {
                self.in_table_cell = true;
                self.table_cell_buf.clear();
            }
            _ => {}
        }
    }

    fn end(&mut self, ui: &mut Ui, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush(ui);
                ui.add_space(2.0);
            }
            TagEnd::Heading(_) => {
                self.flush(ui);
                if let Some(orig) = self.font_size_stack.pop() {
                    self.font_size = orig;
                }
                ui.add_space(4.0);
            }
            TagEnd::CodeBlock => {
                if let Some(code) = self.code_block_buf.take() {
                    if !code.is_empty() {
                        ui.add_space(4.0);
                        ui.add(
                            Label::new(
                                RichText::new(code.trim_end())
                                    .monospace()
                                    .size(self.font_size - 1.0)
                                    .color(Color32::from_rgb(200, 200, 200)),
                            )
                            .selectable(true)
                            .wrap(),
                        );
                        ui.add_space(4.0);
                    }
                }
            }
            TagEnd::Emphasis => {
                self.close_cur_run();
                self.italic = false;
            }
            TagEnd::Strong => {
                self.close_cur_run();
                self.bold = false;
            }
            TagEnd::Strikethrough => {
                self.close_cur_run();
                self.strike = false;
            }
            TagEnd::BlockQuote(_) => {
                self.flush(ui);
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush(ui);
                self.list_depth = self.list_depth.saturating_sub(1);
            }
            TagEnd::Item => {
                self.flush(ui);
            }
            TagEnd::Link => {
                self.link_active = false;
                let text = std::mem::take(&mut self.link_buf);
                let url = std::mem::take(&mut self.link_url);
                let display = if text.is_empty() { url.clone() } else { text };
                ui.hyperlink_to(display, url);
            }
            TagEnd::Image => {}
            TagEnd::Table => {
                self.flush(ui);
                self.render_table(ui);
            }
            TagEnd::TableHead => {
                self.table_rows
                    .push(std::mem::take(&mut self.table_cur_row));
                self.table_is_head = false;
            }
            TagEnd::TableRow => {
                self.table_rows
                    .push(std::mem::take(&mut self.table_cur_row));
            }
            TagEnd::TableCell => {
                self.table_cur_row
                    .push(std::mem::take(&mut self.table_cell_buf));
                self.in_table_cell = false;
            }
            _ => {}
        }
    }

    fn render_table(&mut self, ui: &mut Ui) {
        if self.table_rows.is_empty() {
            return;
        }
        self.table_counter = self.table_counter.wrapping_add(1);
        let id = egui::Id::new(format!("md_table_{}", self.table_counter));
        let fs = self.font_size;
        egui::Grid::new(id)
            .striped(true)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for (i, row) in self.table_rows.iter().enumerate() {
                    for cell in row {
                        let rt = if i == 0 {
                            RichText::new(cell).strong().size(fs)
                        } else {
                            RichText::new(cell).size(fs)
                        };
                        ui.add(Label::new(rt).selectable(true));
                    }
                    ui.end_row();
                }
            });
        self.table_rows.clear();
        ui.add_space(4.0);
    }
}

pub fn render_markdown(ui: &mut Ui, text: &str, font_size: f32) {
    let parser = Parser::new(text);
    let mut st = State::new(font_size);

    for event in parser {
        match event {
            Event::Start(tag) => st.start(ui, tag),
            Event::End(tag) => st.end(ui, tag),
            Event::Text(t) => st.push_text(&t),
            Event::Code(t) => {
                if st.code_block_buf.is_some() {
                    if let Some(buf) = st.code_block_buf.as_mut() {
                        buf.push_str(&t);
                    }
                } else {
                    st.close_cur_run();
                    st.push_run_text(t.into_string(), true);
                }
            }
            Event::SoftBreak | Event::HardBreak => st.push_text("\n"),
            Event::TaskListMarker(checked) => {
                st.push_text(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(r) => st.push_text(&format!("[^{}]", r)),
            _ => {}
        }
    }
    st.flush(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_text_into_cur() {
        let mut st = State::new(14.0);
        st.push_text("hello ");
        st.push_text("world");
        assert_eq!(st.cur, "hello world");
    }

    #[test]
    fn test_close_cur_run_promotes_to_runs() {
        let mut st = State::new(14.0);
        st.push_text("abc");
        st.close_cur_run();
        assert!(st.cur.is_empty());
        assert_eq!(st.runs.len(), 1);
        assert_eq!(st.runs[0].text, "abc");
        assert!(!st.runs[0].code);
    }

    #[test]
    fn test_inline_code_run_flagged_as_code() {
        let mut st = State::new(14.0);
        st.push_run_text("x".into(), true);
        assert_eq!(st.runs.len(), 1);
        assert!(st.runs[0].code);
    }

    #[test]
    fn test_push_text_routes_into_code_block_buf() {
        let mut st = State::new(14.0);
        st.code_block_buf = Some(String::new());
        st.push_text("let x = 1");
        assert!(st.cur.is_empty());
        assert_eq!(st.code_block_buf.as_deref().unwrap(), "let x = 1");
    }

    #[test]
    fn test_push_text_routes_into_link_buf() {
        let mut st = State::new(14.0);
        st.link_active = true;
        st.push_text("a link");
        assert_eq!(st.link_buf, "a link");
    }

    #[test]
    fn test_push_text_routes_into_table_cell() {
        let mut st = State::new(14.0);
        st.in_table_cell = true;
        st.push_text("cell");
        assert_eq!(st.table_cell_buf, "cell");
    }
}

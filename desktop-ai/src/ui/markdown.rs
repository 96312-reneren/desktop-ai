//! `Markdown` 到 `egui` 控件的渲染器，基于 `pulldown-cmark` 解析事件流。
//!
//! 由聊天视图展示消息时调用 [`render_markdown`]：行内事件归并为 `Run` 序列后
//! 一次性绘制，代码块、表格、引用与列表分别特殊处理。
//!
//! 安全边界：链接必须通过 [`is_safe_link_url`] 的 scheme 白名单（`http`/`https`/
//! `mailto`）才渲染为可点击超链接，其余（`file://`、`javascript:`、`data:`、UNC
//! `\\host` 及任意操作系统注册协议等）一律降级为不可点击纯文本，绝不把任意外部
//! scheme 交给操作系统协议处理器；图片仅输出占位文本，不发起任何远程拉取。

use egui::{Color32, Label, RichText, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// 一段带行内样式的文本，是渲染的最小单元。
///
/// `text` 为内容，其余布尔量分别标记粗体、斜体、删除线与行内代码。
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

/// `Markdown` 渲染过程中的可变状态：行内样式、文本缓冲、缩进深度与表格收集。
struct State {
    /// 当前基础字号，标题会在其基础上临时放大。
    font_size: f32,
    /// 当前是否处于粗体行内样式。
    bold: bool,
    /// 当前是否处于斜体行内样式。
    italic: bool,
    /// 当前是否处于删除线行内样式。
    strike: bool,
    /// 待绘制的文本块序列，由 [`Self::flush`] 消费。
    runs: Vec<Run>,
    /// 当前正在拼接的普通行内文本缓冲。
    cur: String,

    /// 代码块文本缓冲；为 `Some` 表示正处于代码块内。
    code_block_buf: Option<String>,

    /// 是否正在解析链接的显示文本。
    link_active: bool,
    /// 链接显示文本缓冲。
    link_buf: String,
    /// 链接目标地址。
    link_url: String,

    /// 列表嵌套深度，用于计算项目符号缩进。
    list_depth: u32,
    /// 引用嵌套深度，用于计算左侧缩进。
    blockquote_depth: u32,
    /// 标题字号恢复栈：进入标题时压入、离开时弹出。
    font_size_stack: Vec<f32>,

    /// 已收集的表格行（每行含多个单元格）。
    table_rows: Vec<Vec<String>>,
    /// 当前处理的行是否为表头。
    table_is_head: bool,
    /// 当前正在收集的表格行。
    table_cur_row: Vec<String>,
    /// 是否正在收集某个表格单元格。
    in_table_cell: bool,
    /// 当前单元格文本缓冲。
    table_cell_buf: String,
    /// 表格自增计数，用于生成唯一网格 `Id`。
    table_counter: u32,
}

impl State {
    /// 以给定基础字号创建空的渲染状态。
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

    /// 按当前样式标记构造 `Run` 并追加到待渲染序列；空文本直接忽略。
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

    /// 把 `cur` 中累积的普通文本固化为一个非代码 `Run` 并清空缓冲。
    fn close_cur_run(&mut self) {
        if !self.cur.is_empty() {
            let text = std::mem::take(&mut self.cur);
            self.push_run_text(text, false);
        }
    }

    /// 绘制并清空当前累积的 `Run`：按引用深度缩进，逐段套用样式后以可选中标签显示。
    ///
    /// 代码 `Run` 使用等宽字体与深色底；无内容时直接返回。
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

    /// 按当前上下文把文本追加到正确的缓冲：代码块、链接文本、表格单元格或普通行内文本。
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

    /// 处理 `pulldown-cmark` 的开始标签：切换行内样式、压入字号或初始化代码块/表格缓冲。
    ///
    /// 块级标签（标题、代码块、引用、列表、表格等）会先调用 [`Self::flush`] 收尾上一段。
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

    /// 处理 `pulldown-cmark` 的结束标签：收尾代码块/表格并弹出样式与字号状态。
    ///
    /// 链接结束经 [`is_safe_link_url`] 过滤，决定渲染为超链接还是纯文本；图片的空
    /// 处理见 `TagEnd::Image` 分支说明。
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
                        let code = code.trim_end().to_string();
                        ui.add_space(4.0);
                        egui::Frame::new()
                            .fill(Color32::from_rgb(30, 30, 30))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 6,
                                bottom: 8,
                            })
                            .show(ui, |ui| {
                                // ── Copy button row ──
                                ui.horizontal(|ui| {
                                    ui.add_space(ui.available_width() - 50.0);
                                    let copy_btn = ui.add(
                                        egui::Button::new(
                                            RichText::new("📋 复制")
                                                .size(self.font_size - 3.0)
                                                .color(Color32::from_rgb(160, 160, 160)),
                                        )
                                        .fill(Color32::from_rgb(45, 45, 45))
                                        .corner_radius(egui::CornerRadius::same(6)),
                                    );
                                    if copy_btn.clicked() {
                                        ui.ctx().copy_text(code.clone());
                                    }
                                    if copy_btn.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                });
                                ui.add_space(2.0);
                                ui.add(
                                    Label::new(
                                        RichText::new(&code)
                                            .monospace()
                                            .size(self.font_size - 1.0)
                                            .color(Color32::from_rgb(200, 200, 200)),
                                    )
                                    .selectable(true)
                                    .wrap(),
                                );
                            });
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
                // 只允许可控 scheme（http/https/mailto）进入可点击超链接；
                // 其余（file://、javascript:、data:、UNC \\host、任意 OS 注册
                // 协议等）一律降级为不可点击的纯文本，避免把"打开链接"交给
                // 操作系统的协议处理器。
                if is_safe_link_url(&url) {
                    ui.hyperlink_to(display, url);
                } else if !display.is_empty() {
                    ui.add(
                        Label::new(RichText::new(display).size(self.font_size)).selectable(true),
                    );
                }
            }
            // 图片不做任何拉取：`Tag::Image` 开始时已把地址写成占位文本，避免
            // 远程加载带来的 SSRF、追踪与隐私泄露，故结束标签为空实现。
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

    /// 将收集到的表格行渲染为 `egui` 网格：首行加粗作表头，随后清空缓冲并留白。
    ///
    /// 用自增计数器生成唯一 `Id`，避免同一帧内多个表格共用网格状态。
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

/// 渲染 `Markdown` 文本：解析事件流并驱动 [`State`]，最后绘制剩余文本。
///
/// `font_size` 为基础字号，标题会在此基础上放大；函数就地绘制，无返回值。
pub(crate) fn render_markdown(ui: &mut Ui, text: &str, font_size: f32) {
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

/// 判断 URL 是否允许渲染为可点击链接（红队缓解：scheme 白名单）。
/// 仅放行 http://、https://、mailto:（大小写不敏感）；拒绝包含控制字符
/// 或空白字符的 URL，防止 "java\tscript:" 这类混淆绕过。
fn is_safe_link_url(raw: &str) -> bool {
    let url = raw.trim();
    if url.is_empty() || url.contains(|c: char| c.is_control()) || url.contains(char::is_whitespace)
    {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
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

    #[test]
    fn test_safe_link_url_accepts_http_https_mailto() {
        assert!(is_safe_link_url("http://example.com/a?b=c#d"));
        assert!(is_safe_link_url("https://example.com"));
        assert!(is_safe_link_url("mailto:user@example.com"));
        assert!(is_safe_link_url("HTTPS://EXAMPLE.COM/Path"));
        assert!(is_safe_link_url("  https://example.com  "));
    }

    #[test]
    fn test_safe_link_url_rejects_dangerous_schemes() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "vbscript:msgbox(1)",
            "data:text/html,<script>alert(1)</script>",
            "\\\\attacker\\share\\payload.exe",
            "//attacker.com/redirect",
            "www.example.com",  // 无 scheme
            "example.com/path", // 相对路径
            "tel:+1234567890",  // OS 注册协议
            "ms-settings:default",
            "ftp://example.com/file",
        ] {
            assert!(!is_safe_link_url(url), "应当拒绝: {url}");
        }
    }

    #[test]
    fn test_safe_link_url_rejects_control_and_whitespace() {
        assert!(!is_safe_link_url(""));
        assert!(!is_safe_link_url("http://example.com/x\n.html"));
        assert!(!is_safe_link_url("http://a b.com"));
        assert!(!is_safe_link_url("java\tscript:alert(1)"));
        assert!(!is_safe_link_url("\u{0}https://evil"));
    }
}

use egui::{Color32, RichText, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

pub fn render_markdown(ui: &mut Ui, text: &str, font_size: f32) {
    let parser = Parser::new(text);
    let mut in_code_block = false;
    let mut code_text = String::new();
    let mut list_depth = 0u32;
    let mut current_text = String::new();
    let mut current_bold = false;
    let mut current_code = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::CodeBlock(_) => {
                    flush_text(ui, &mut current_text, current_bold, current_code, font_size, list_depth);
                    in_code_block = true;
                    code_text.clear();
                }
                Tag::Paragraph if list_depth == 0 => {}
                Tag::Heading { level, .. } => {
                    let size = match level {
                        pulldown_cmark::HeadingLevel::H1 => font_size + 1.4,
                        pulldown_cmark::HeadingLevel::H2 => font_size + 1.1,
                        _ => font_size + 0.8,
                    };
                    flush_text(ui, &mut current_text, current_bold, current_code, size, list_depth);
                }
                Tag::Strong => current_bold = true,
                Tag::Item => { ui.add_space(4.0); }
                Tag::List(_) => list_depth += 1,
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::CodeBlock => {
                    if in_code_block && !code_text.is_empty() {
                        ui.add_space(4.0);
                        let text = RichText::new(&code_text)
                            .monospace()
                            .size(font_size - 1.0)
                            .color(Color32::from_rgb(200, 200, 200));
                        ui.add(egui::Label::new(text).wrap());
                        ui.add_space(4.0);
                    }
                    in_code_block = false;
                }
                TagEnd::Strong => {
                    flush_text(ui, &mut current_text, true, current_code, font_size, list_depth);
                    current_bold = false;
                }
                TagEnd::List(_) => { list_depth = list_depth.saturating_sub(1); }
                TagEnd::Heading(_) => { ui.add_space(4.0); }
                TagEnd::Paragraph => { ui.add_space(2.0); }
                _ => {}
            },
            Event::Text(t) | Event::Code(t) => {
                if in_code_block {
                    code_text.push_str(&t);
                } else {
                    current_text.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                current_text.push('\n');
            }
            _ => {}
        }
    }

    flush_text(ui, &mut current_text, current_bold, current_code, font_size, list_depth);
}

fn flush_text(ui: &mut Ui, text: &mut String, bold: bool, code: bool, size: f32, indent: u32) {
    if text.is_empty() { return; }
    let trimmed = text.trim_end();
    if trimmed.is_empty() { text.clear(); return; }

    let mut rt = RichText::new(trimmed).size(size);
    if bold { rt = rt.strong(); }
    if code {
        rt = rt.monospace().background_color(Color32::from_rgb(50, 50, 50))
            .color(Color32::from_rgb(220, 220, 100));
    }
    let indent_px = indent as f32 * 16.0;
    if indent_px > 0.0 { ui.add_space(indent_px); }
    ui.label(rt);
    text.clear();
}

use std::time::Duration;

pub struct CrawledPage {
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_size: usize,
}

pub fn crawl_url(url: &str) -> Result<CrawledPage, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) DesktopAI/5.7")
        .build()
        .map_err(|e| format!("创建连接失败: {}", e))?;

    let response = client.get(url).send()
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }

    let content_type = response.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let raw = response.text()
        .map_err(|e| format!("读取响应失败: {}", e))?;

    if raw.len() > 3_000_000 {
        return Err("页面过大(>3MB)，请尝试更小的页面".into());
    }

    let (title, text) = if content_type.contains("text/html") || content_type.is_empty() {
        crate::cleaner::clean_text(&raw, "html")
    } else {
        // Treat as plain text
        crate::cleaner::clean_text(&raw, "text")
    };

    if text.len() < 50 {
        return Err("提取的文本内容过少(<50字符)，可能不是文本页面".into());
    }

    Ok(CrawledPage {
        url: url.to_string(),
        title: if title.is_empty() { url.to_string() } else { title },
        text_size: text.len(),
        text,
    })
}

pub fn crawl_multiple(urls: &[String]) -> Vec<Result<CrawledPage, String>> {
    urls.iter().map(|url| crawl_url(url)).collect()
}

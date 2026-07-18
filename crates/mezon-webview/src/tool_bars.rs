use wry::WebView;

pub fn reload(webview: &WebView) {
    let _ = webview.evaluate_script("location.reload()");
}

pub fn go_back(webview: &WebView) {
    let _ = webview.evaluate_script("history.back()");
}

pub fn go_forward(webview: &WebView) {
    let _ = webview.evaluate_script("history.forward()");
}

pub fn current_url(webview: Option<&WebView>, fallback: &str) -> String {
    webview
        .and_then(|webview| webview.url().ok())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

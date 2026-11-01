//! Open saved web links without interpreting them as commands or local files.
pub(crate) fn web_url(value: &str) -> Result<url::Url, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Add a website link first.".into());
    }
    if value.starts_with('-') {
        return Err("Enter a valid website link.".into());
    }
    let candidate = if value.contains("://") || value.contains(':') {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url = url::Url::parse(&candidate).map_err(|_| "Enter a valid website link.".to_owned())?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use an http or https website link without embedded credentials.".into());
    }
    Ok(url)
}

pub fn open(value: &str) -> Result<(), String> {
    let url = web_url(value)?;
    launch(url.as_str())
}

#[cfg(target_os = "macos")]
fn launch(url: &str) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|_| "Could not open your browser.".to_owned())?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open this website.".into())
    }
}

#[cfg(target_os = "ios")]
fn launch(url: &str) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSDictionary, NSString, NSURL};
    use objc2_ui_kit::UIApplication;
    let main = MainThreadMarker::new().ok_or("Open links from the app's main screen.")?;
    let url = NSURL::URLWithString(&NSString::from_str(url)).ok_or("Invalid website link.")?;
    // The empty dictionary has no incorrectly typed option values; UIKit owns the request.
    unsafe {
        UIApplication::sharedApplication(main).openURL_options_completionHandler(
            &url,
            &NSDictionary::new(),
            None,
        );
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn launch(_: &str) -> Result<(), String> {
    Err("Copy the link to open it in your browser.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_web_links_and_normalizes_bare_domains() {
        assert_eq!(
            web_url("example.com/login").unwrap().as_str(),
            "https://example.com/login"
        );
        assert_eq!(
            web_url(" https://example.com/ ").unwrap().host_str(),
            Some("example.com")
        );
        assert!(web_url("http://localhost:8080/login").is_ok());
    }
    #[test]
    fn rejects_executable_local_and_credential_bearing_links() {
        for value in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hello",
            "https://name:secret@example.com",
            "--help",
            "not a domain",
        ] {
            assert!(web_url(value).is_err(), "accepted {value}");
        }
    }
}

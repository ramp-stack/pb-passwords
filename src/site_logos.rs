//! Optional, origin-only site icons. Private login fields never leave the app.
use chk::{
    AvatarContent,
    image::{self, io::Reader as ImageReader},
};
use prism::hardware::Image;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::poll_fn,
    io::{self, Cursor, Read},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Poll, Waker},
    time::{Duration, Instant},
};

const MAX_DOMAINS: usize = 256;
const MAX_ICON_BYTES: usize = 512 * 1024;

#[derive(Debug, Default)]
struct Cache {
    // None records a pending or failed lookup, so a missing favicon does not
    // cause another request every time a page is opened.
    icons: HashMap<String, Option<AvatarContent>>,
    queued: VecDeque<String>,
    waiter: Option<Waker>,
}

#[derive(Clone, Debug, Default)]
pub struct SiteLogos {
    cache: Rc<RefCell<Cache>>,
    task: Rc<RefCell<Option<LogoTask>>>,
}

#[derive(Debug, Default)]
pub struct LogoResults(Vec<(String, Option<Vec<u8>>)>);

// Network work is independent of the UI framework.
#[derive(Clone, Debug)]
struct LogoTask(Arc<Mutex<LogoTaskState>>);

#[derive(Debug, Default)]
struct LogoTaskState {
    result: Option<LogoResults>,
    finished: bool,
    waiter: Option<Waker>,
}

impl LogoTask {
    fn spawn(work: impl FnOnce() -> LogoResults + Send + 'static) -> Self {
        let state = Arc::new(Mutex::new(LogoTaskState::default()));
        let worker = state.clone();
        std::thread::spawn(move || {
            let result = work();
            let waiter = {
                let mut state = worker.lock().unwrap();
                state.result = Some(result);
                state.finished = true;
                state.waiter.take()
            };
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        });
        Self(state)
    }

    async fn next(&mut self) -> Option<LogoResults> {
        poll_fn(|ctx| {
            let mut state = self.0.lock().unwrap();
            if state.finished {
                Poll::Ready(state.result.take())
            } else {
                state.waiter = Some(ctx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

impl SiteLogos {
    /// Only call for saved logins. Unaccepted incoming logins must not trigger
    /// network requests merely because someone sent them to this app.
    pub fn get(&self, name: &str, link: &str) -> Option<AvatarContent> {
        let domain = domain_for(name, link)?;
        let mut cache = self.cache.borrow_mut();
        if let Some(icon) = cache.icons.get(&domain) {
            return icon.clone();
        }
        if !cfg!(test) && cache.icons.len() < MAX_DOMAINS {
            cache.icons.insert(domain.clone(), None);
            cache.queued.push_back(domain);
            if let Some(waiter) = cache.waiter.take() {
                waiter.wake();
            }
        }
        None
    }

    pub async fn next(&mut self) -> LogoResults {
        if self.task.borrow().is_none() {
            let cache = self.cache.clone();
            let domains = poll_fn(move |ctx| {
                let mut cache = cache.borrow_mut();
                if cache.queued.is_empty() {
                    cache.waiter = Some(ctx.waker().clone());
                    return Poll::Pending;
                }
                Poll::Ready(
                    (0..4)
                        .filter_map(|_| cache.queued.pop_front())
                        .collect::<Vec<_>>(),
                )
            })
            .await;
            *self.task.borrow_mut() = Some(LogoTask::spawn(move || {
                let workers: Vec<_> = domains
                    .into_iter()
                    .map(|domain| {
                        std::thread::spawn(move || {
                            let bytes = fetch_icon(&domain);
                            (domain, bytes)
                        })
                    })
                    .collect();
                LogoResults(
                    workers
                        .into_iter()
                        .filter_map(|worker| worker.join().ok())
                        .collect(),
                )
            }));
        }
        let mut task = self.task.borrow().as_ref().unwrap().clone();
        task.next().await.unwrap_or_default()
    }

    pub fn finish(&mut self, results: LogoResults) {
        *self.task.borrow_mut() = None;
        let mut cache = self.cache.borrow_mut();
        for (domain, bytes) in results.0 {
            let icon = bytes
                .and_then(|bytes| Image::read(bytes.as_slice()).ok())
                .map(AvatarContent::image);
            cache.icons.insert(domain, icon);
        }
    }
}

/// A supplied link takes precedence. Names are used only when they identify a
/// domain or a recognized service; an email username is never sent or guessed.
fn domain_for(name: &str, link: &str) -> Option<String> {
    if !link.trim().is_empty() {
        return domain_from_url(link);
    }
    let name = name.trim().to_lowercase();
    if name.contains('@') {
        return None;
    }
    if !name.chars().any(char::is_whitespace) && (name.contains('.') || name.contains("://")) {
        return domain_from_url(&name);
    }
    // Recognize service words inside labels such as “Ella’s Gmail” locally.
    // Exact word boundaries avoid guessing from names like “gmailish”. If a
    // label names different services, leave it to an explicit saved link.
    let mut found = None;
    for word in name.split(|character: char| !character.is_alphanumeric()) {
        let domain = match word {
            "amazon" => "amazon.com",
            "apple" | "icloud" => "apple.com",
            "google" => "google.com",
            "gmail" => "gmail.com",
            "youtube" => "youtube.com",
            "facebook" => "facebook.com",
            "instagram" => "instagram.com",
            "linkedin" => "linkedin.com",
            "microsoft" | "office" | "onedrive" => "microsoft.com",
            "outlook" | "hotmail" => "outlook.com",
            "github" => "github.com",
            "gitlab" => "gitlab.com",
            "dropbox" => "dropbox.com",
            "figma" => "figma.com",
            "notion" => "notion.so",
            "spotify" => "spotify.com",
            "netflix" => "netflix.com",
            "slack" => "slack.com",
            "discord" => "discord.com",
            "zoom" => "zoom.us",
            "reddit" => "reddit.com",
            "ebay" => "ebay.com",
            "paypal" => "paypal.com",
            "target" => "target.com",
            "walmart" => "walmart.com",
            "ikea" => "ikea.com",
            "adobe" => "adobe.com",
            "canva" => "canva.com",
            "openai" => "openai.com",
            "chatgpt" => "chatgpt.com",
            _ => continue,
        };
        if found.is_some_and(|previous| previous != domain) {
            return None;
        }
        found = Some(domain);
    }
    found.map(str::to_owned)
}

fn domain_from_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.contains('@') || value.chars().any(char::is_whitespace) {
        return None;
    }
    let candidate = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url = url::Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let url::Host::Domain(domain) = url.host()? else {
        return None;
    };
    public_domain(domain).then(|| domain.trim_end_matches('.').to_owned())
}

fn public_domain(domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    domain.contains('.')
        && domain.split('.').all(|label| !label.is_empty())
        && ![
            "local",
            "localhost",
            "internal",
            "test",
            "invalid",
            "example",
            "onion",
            "home",
            "lan",
        ]
        .iter()
        .any(|suffix| domain == *suffix || domain.ends_with(&format!(".{suffix}")))
        && !["example.com", "example.net", "example.org"]
            .iter()
            .any(|suffix| domain == *suffix || domain.ends_with(&format!(".{suffix}")))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && a != 0
                && a < 240
                && !(a == 100 && (64..128).contains(&b))
                && !(a == 192 && b == 0 && (c == 0 || c == 2))
                && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ip));
            }
            let segments = ip.segments();
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !(segments[0] == 0x2001 && segments[1] == 0xdb8)
        }
    }
}

fn resolve_public(name: &str) -> io::Result<Vec<SocketAddr>> {
    let addresses: Vec<_> = name
        .to_socket_addrs()?
        .filter(|address| public_ip(address.ip()))
        .collect();
    if addresses.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "No public site address",
        ))
    } else {
        Ok(addresses)
    }
}

fn fetch_icon(domain: &str) -> Option<Vec<u8>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .redirects(0)
        .resolver(resolve_public)
        .build();
    let started = Instant::now();
    // Gmail's app artwork lives on Google's static asset host rather than
    // gmail.com. Only this explicit public asset is used; arbitrary redirects
    // still have to stay on the originally requested site.
    if let Some((host, path)) = known_icon_source(domain)
        && let Some(bytes) = fetch_image(&agent, host, path)
        && let Some(icon) = thumbnail(bytes)
    {
        return Some(icon);
    }
    let mut best: Option<(u32, Vec<u8>)> = None;
    // Touch icons are intended for larger surfaces. The site's ordinary
    // favicon can be only 16–32 pixels and looks blurred in a login avatar.
    for path in [
        "/apple-touch-icon.png",
        "/apple-touch-icon-precomposed.png",
        "/favicon.png",
        "/favicon.ico",
    ] {
        if started.elapsed() > Duration::from_secs(12) {
            break;
        }
        if let Some(bytes) = fetch_image(&agent, domain, path)
            && let Some((width, height)) = icon_dimensions(&bytes)
            && let Some(icon) = thumbnail(bytes)
        {
            let quality = width.min(height);
            if quality >= 128 {
                return Some(icon);
            }
            if best.as_ref().is_none_or(|(size, _)| quality > *size) {
                best = Some((quality, icon));
            }
        }
    }
    best.map(|(_, icon)| icon)
}

fn known_icon_source(domain: &str) -> Option<(&'static str, &'static str)> {
    match domain.trim_start_matches("www.") {
        "gmail.com" | "mail.google.com" | "googlemail.com" => Some((
            "www.gstatic.com",
            "/images/branding/product/2x/gmail_2020q4_512dp.png",
        )),
        _ => None,
    }
}

fn fetch_image(agent: &ureq::Agent, domain: &str, path: &str) -> Option<Vec<u8>> {
    let mut url = url::Url::parse(&format!("https://{domain}{path}")).ok()?;
    for _ in 0..3 {
        let response = agent
            .get(url.as_str())
            .set("Accept", "image/*")
            .call()
            .ok()?;
        if (300..400).contains(&response.status()) {
            let target = url.join(response.header("Location")?).ok()?;
            if !same_site_icon(&target, domain) {
                return None;
            }
            url = target;
            continue;
        }
        if response.status() != 200 {
            return None;
        }
        if response
            .header("Content-Length")
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|length| length > MAX_ICON_BYTES)
        {
            return None;
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_ICON_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        return (!bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES).then_some(bytes);
    }
    None
}

fn same_site_icon(url: &url::Url, original: &str) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            public_domain(host)
                && host.trim_start_matches("www.") == original.trim_start_matches("www.")
        })
}

fn icon_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    Some((width, height))
}

fn thumbnail(bytes: Vec<u8>) -> Option<Vec<u8>> {
    icon_dimensions(&bytes)?;
    let mut reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let content = if decoded.width() > 256 || decoded.height() > 256 {
        decoded.thumbnail(256, 256)
    } else {
        decoded
    }
    .into_rgba8();
    // A 64-point avatar occupies about 128 device pixels. Give small icons a
    // transparent square canvas instead of magnifying their pixels; larger
    // icons are only reduced. Padding also preserves non-square logo shapes.
    let side = content.width().max(content.height()).max(128);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &content,
        i64::from((side - content.width()) / 2),
        i64::from((side - content.height()) / 2),
    );
    let image = image::DynamicImage::ImageRgba8(canvas);
    let mut result = Cursor::new(Vec::new());
    image
        .write_to(&mut result, image::ImageOutputFormat::Png)
        .ok()?;
    Some(result.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_batches_cache_icons_and_return_to_idle() {
        use std::{future::Future, task::Context};

        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(32, 32)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        let mut logos = SiteLogos::default();
        *logos.task.borrow_mut() = Some(LogoTask::spawn(move || {
            LogoResults(vec![("github.com".into(), Some(bytes.into_inner()))])
        }));
        // CHK clones watchers when another source refreshes the page.
        let mut refreshed = logos.clone();
        let result = {
            let mut next = Box::pin(refreshed.next());
            let mut context = Context::from_waker(Waker::noop());
            let started = Instant::now();
            loop {
                if let Poll::Ready(result) = next.as_mut().poll(&mut context) {
                    break result;
                }
                assert!(started.elapsed() < Duration::from_secs(2));
                std::thread::yield_now();
            }
        };
        refreshed.finish(result);
        assert!(logos.task.borrow().is_none());
        assert!(logos.get("GitHub", "").is_some());
        assert!(
            logos
                .clone()
                .get("", "https://github.com/private/path")
                .is_some()
        );
        assert!(logos.cache.borrow().queued.is_empty());
        let mut next = Box::pin(logos.next());
        assert!(
            next.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }

    #[test]
    fn infers_only_site_identity_and_prefers_links() {
        assert_eq!(domain_for("Amazon login", ""), Some("amazon.com".into()));
        assert_eq!(domain_for("Target.com", ""), Some("target.com".into()));
        assert_eq!(
            domain_for("Amazon", "https://github.com/login?token=private#fragment"),
            Some("github.com".into())
        );
        assert_eq!(domain_for("me@gmail.com", ""), None);
        assert_eq!(domain_for("my private work account", ""), None);
    }

    #[test]
    fn personalized_labels_match_whole_service_words_only() {
        for name in [
            "Ella's Gmail",
            "Ella’s Gmail",
            "ELLA’S GMAIL (personal)",
            "Gmail — work account",
            "Ella B. Gmail",
        ] {
            assert_eq!(domain_for(name, ""), Some("gmail.com".into()), "{name}");
        }
        assert_eq!(
            domain_for("Family Netflix account", ""),
            Some("netflix.com".into())
        );
        assert_eq!(
            domain_for("Work Microsoft Office", ""),
            Some("microsoft.com".into())
        );
        assert_eq!(
            domain_for("Ella’s Gmail", "https://github.com/login?private=detail"),
            Some("github.com".into())
        );
        for name in [
            "gmailish",
            "mygmail",
            "gmail2",
            "Gmail or Outlook",
            "Ella's Gmail me@gmail.com",
        ] {
            assert_eq!(domain_for(name, ""), None, "{name}");
        }
    }

    #[test]
    fn gmail_uses_only_its_fixed_official_large_icon_source() {
        for domain in [
            "gmail.com",
            "www.gmail.com",
            "mail.google.com",
            "googlemail.com",
        ] {
            assert_eq!(
                known_icon_source(domain),
                Some((
                    "www.gstatic.com",
                    "/images/branding/product/2x/gmail_2020q4_512dp.png"
                ))
            );
        }
        for domain in [
            "google.com",
            "gmail.com.evil.com",
            "other-gmail.com",
            "mail.evil.com",
        ] {
            assert_eq!(known_icon_source(domain), None);
        }
        assert_eq!(
            domain_for("Ella's Gmail", "https://mail.google.com/mail/u/0/#inbox"),
            Some("mail.google.com".into())
        );
    }

    #[test]
    fn rejects_local_or_credential_bearing_destinations() {
        for url in [
            "http://localhost",
            "http://127.0.0.1",
            "https://[::1]",
            "https://router.local",
            "https://example.test",
            "https://user:secret@github.com",
            "https://github.com:8080",
            "file:///private/data",
            "javascript:alert(1)",
        ] {
            assert!(domain_for("Amazon", url).is_none(), "accepted {url}");
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "accepted {ip}");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn icon_redirects_stay_on_the_public_https_site() {
        assert!(same_site_icon(
            &url::Url::parse("https://www.github.com/favicon.ico").unwrap(),
            "github.com"
        ));
        for target in [
            "https://elsewhere.com/favicon.ico",
            "http://github.com/favicon.ico",
            "https://user:secret@github.com/favicon.ico",
            "https://127.0.0.1/favicon.ico",
        ] {
            assert!(!same_site_icon(
                &url::Url::parse(target).unwrap(),
                "github.com"
            ));
        }
    }

    #[test]
    fn downloaded_images_are_bounded_and_normalized() {
        assert!(thumbnail(b"not an image".to_vec()).is_none());
        let mut icon = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(32, 32)
            .write_to(&mut icon, image::ImageOutputFormat::Ico)
            .unwrap();
        assert!(thumbnail(icon.into_inner()).is_some());
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2048, 1)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        assert!(thumbnail(bytes.into_inner()).is_none());
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(512, 256)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        let result = thumbnail(bytes.into_inner()).unwrap();
        assert_eq!(
            ImageReader::new(Cursor::new(result))
                .with_guessed_format()
                .unwrap()
                .into_dimensions()
                .unwrap(),
            (256, 256)
        );
    }

    #[test]
    fn small_icons_are_padded_without_upscaling_and_touch_icons_keep_detail() {
        for size in [32, 180] {
            let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                size,
                size,
                image::Rgba([200, 20, 10, 255]),
            ));
            let mut bytes = Cursor::new(Vec::new());
            image
                .write_to(&mut bytes, image::ImageOutputFormat::Png)
                .unwrap();
            let result = thumbnail(bytes.into_inner()).unwrap();
            let decoded = image::load_from_memory(&result).unwrap().into_rgba8();
            assert_eq!(decoded.dimensions(), (size.max(128), size.max(128)));
            assert_eq!(
                decoded.pixels().filter(|pixel| pixel.0[3] != 0).count(),
                (size * size) as usize
            );
            if size < 128 {
                assert_eq!(decoded.get_pixel(0, 0).0[3], 0);
                assert_eq!(decoded.get_pixel(64, 64).0, [200, 20, 10, 255]);
            }
        }
    }
}

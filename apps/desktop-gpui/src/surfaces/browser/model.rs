//! Client-owned surface identity. Closing a tab releases its native view and store.
use url::{Host, Url};

pub const MAX_BROWSER_TABS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Default,
    /// Each tab owns a separate non-persistent WebKit data store.
    Incognito,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Navigation {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub error: Option<String>,
}

/// HTTP(S) only: address-bar text is never evaluated as script or opened as a file.
pub fn address(text: &str) -> Result<Url, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a preview URL".into());
    }
    let text = if text.contains("://") {
        text.to_owned()
    } else if text.starts_with("localhost:")
        || text.starts_with("127.0.0.1:")
        || text.starts_with("[::1]:")
    {
        format!("http://{text}")
    } else {
        format!("https://{text}")
    };
    let url = Url::parse(&text).map_err(|error| format!("Invalid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use an HTTP or HTTPS URL without embedded credentials".into());
    }
    Ok(url)
}

fn loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(host) => {
            matches!(*host, "localhost" | "0.0.0.0") || host.ends_with(".localhost")
        }
        Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

fn private(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(host) => {
            host.ends_with(".ts.net") || host.ends_with(".local") || !host.contains('.')
        }
        Host::Ipv4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
        }
        Host::Ipv6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

/// Match the web's direct-private-network preview route. Public/relay endpoints
/// cannot proxy arbitrary ports; never silently load the client's localhost.
pub fn preview_address(text: &str, environment_http_url: &str) -> Result<Url, String> {
    let mut url = address(text)?;
    if !loopback(&url.host().expect("address validates host")) {
        return Ok(url);
    }
    let environment = Url::parse(environment_http_url).map_err(|_| "Environment URL is invalid")?;
    let host = environment.host().ok_or("Environment URL has no host")?;
    if !private(&host) && !loopback(&host) {
        return Err("Remote preview ports need an authenticated preview gateway or a directly reachable private-network environment".into());
    }
    let host = if loopback(&host) {
        "localhost".to_owned()
    } else {
        host.to_string()
    };
    url.set_host(Some(&host))
        .map_err(|_| "Environment preview host is invalid")?;
    Ok(url)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: u64,
    pub label: String,
    pub profile: Profile,
    renamed: bool,
}

#[derive(Default)]
pub struct Tabs {
    pub entries: Vec<Tab>,
    /// None means the built-in Inspector/Changes/Agents selection owns the body.
    pub active: Option<u64>,
    next: u64,
}

impl Tabs {
    pub fn add(&mut self, profile: Profile) -> Result<u64, String> {
        if self.entries.len() >= MAX_BROWSER_TABS {
            return Err(format!(
                "Close a browser tab before opening another (limit {MAX_BROWSER_TABS})"
            ));
        }
        self.next += 1;
        let id = self.next;
        self.entries.push(Tab {
            id,
            label: if profile == Profile::Incognito {
                "Incognito"
            } else {
                "Browser"
            }
            .into(),
            profile,
            renamed: false,
        });
        self.active = Some(id);
        Ok(id)
    }
    pub fn activate(&mut self, id: u64) -> bool {
        if !self.entries.iter().any(|tab| tab.id == id) {
            return false;
        }
        self.active = Some(id);
        true
    }
    pub fn close(&mut self, id: u64) -> bool {
        let Some(index) = self.entries.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.entries.remove(index);
        if self.active == Some(id) {
            self.active = self
                .entries
                .get(index)
                .or_else(|| self.entries.last())
                .map(|tab| tab.id);
        }
        true
    }
    pub fn rename(&mut self, id: u64, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 48 {
            return Err("Tab names must contain 1–48 characters".into());
        }
        let tab = self
            .entries
            .iter_mut()
            .find(|tab| tab.id == id)
            .ok_or("Tab was closed")?;
        tab.label = name.into();
        tab.renamed = true;
        Ok(())
    }
    pub fn navigation_title(&mut self, id: u64, title: &str) {
        if let Some(tab) = self.entries.iter_mut().find(|tab| tab.id == id)
            && !tab.renamed
            && !title.trim().is_empty()
        {
            tab.label = title.trim().chars().take(48).collect();
        }
    }
    pub fn move_by(&mut self, id: u64, delta: isize) -> bool {
        let Some(index) = self.entries.iter().position(|tab| tab.id == id) else {
            return false;
        };
        let target = index
            .saturating_add_signed(delta)
            .min(self.entries.len() - 1);
        if index == target {
            return false;
        }
        let tab = self.entries.remove(index);
        self.entries.insert(target, tab);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_tabs_close_neighbor_preserve_identity_rename_and_bound_resources() {
        let mut tabs = Tabs::default();
        let a = tabs.add(Profile::Default).unwrap();
        let b = tabs.add(Profile::Incognito).unwrap();
        let c = tabs.add(Profile::Default).unwrap();
        tabs.navigation_title(a, "Preview page");
        assert_eq!(tabs.entries[0].label, "Preview page");
        tabs.rename(b, " QA ").unwrap();
        tabs.navigation_title(b, "Later document title");
        assert!(tabs.move_by(b, -1));
        assert_eq!(tabs.entries[0].id, b);
        assert_eq!(tabs.entries[0].label, "QA");
        assert_eq!(tabs.entries[0].profile, Profile::Incognito);
        tabs.activate(a);
        tabs.close(a);
        assert_eq!(tabs.active, Some(c));
        assert!(!tabs.activate(a));
        while tabs.entries.len() < MAX_BROWSER_TABS {
            tabs.add(Profile::Default).unwrap();
        }
        assert!(tabs.add(Profile::Default).is_err());
        assert!(tabs.rename(b, " ").is_err());
    }
    #[test]
    fn browser_preview_routes_environment_ports_without_losing_path_query_or_fragment() {
        let url = preview_address(
            "http://localhost:5173/dashboard?q=1#result",
            "http://100.124.111.25:8789",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "http://100.124.111.25:5173/dashboard?q=1#result"
        );
        assert_eq!(
            preview_address("http://127.0.0.1:3000", "http://[::1]:8789")
                .unwrap()
                .host_str(),
            Some("localhost")
        );
        assert!(preview_address("http://localhost:3000", "https://relay.example.com").is_err());
        assert_eq!(
            preview_address("https://example.com/app", "https://relay.example.com")
                .unwrap()
                .as_str(),
            "https://example.com/app"
        );
    }
    #[test]
    fn browser_address_rejects_executable_and_credential_urls() {
        for text in [
            "",
            "javascript://alert(1)",
            "file:///tmp/a",
            "https://user:secret@example.com",
            "data://text/html",
        ] {
            assert!(address(text).is_err(), "{text}");
        }
        assert_eq!(
            address("localhost:3000").unwrap().as_str(),
            "http://localhost:3000/"
        );
        assert_eq!(
            address("example.com").unwrap().as_str(),
            "https://example.com/"
        );
    }
}

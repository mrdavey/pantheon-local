//! Small synchronous client for ChromeDriver's W3C WebDriver endpoints.
//! Shared with the quote acceptance check, which runs outside the test harness.
#![allow(dead_code)]
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    env, fs,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";
const WAIT: Duration = Duration::from_secs(25);
#[derive(Debug)]
struct StaleElement;
impl std::fmt::Display for StaleElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("browser element was replaced")
    }
}
impl std::error::Error for StaleElement {}

pub fn until<T>(description: &str, mut probe: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + WAIT;
    loop {
        let error = match probe() {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => String::new(),
            Err(e) => format!(" ({e:#})"),
        };
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for {description}{error}"
        );
        thread::sleep(Duration::from_millis(90));
    }
}
pub struct Browser {
    client: Client,
    driver: Child,
    base: String,
    profile: Option<tempfile::TempDir>,
}
impl Browser {
    pub fn launch(chrome: Option<&Path>, mobile: bool) -> Result<Self> {
        let driver = env::var_os("CHROMEDRIVER").unwrap_or_else(|| "chromedriver".into());
        let installed = env::var_os("CHROME_BIN");
        Self::launch_with_driver(
            chrome.or(installed.as_ref().map(Path::new)),
            Path::new(&driver),
            mobile,
        )
    }
    pub fn launch_with_driver(chrome: Option<&Path>, driver: &Path, mobile: bool) -> Result<Self> {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let profile = env::var_os("SWARM_DATA_DIR")
            .map(tempfile::tempdir_in)
            .transpose()?;
        let mut driver_command = Command::new(driver);
        // Chromium on macOS consults this variable before the system temp folder.
        if cfg!(target_os = "macos")
            && let Some(data) = env::var_os("SWARM_DATA_DIR")
        {
            driver_command.env("MAC_CHROMIUM_TMPDIR", data);
        }
        let child = driver_command
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .spawn()
            .context("start chromedriver")?;
        let mut me = Self {
            client: Client::builder().no_proxy().timeout(WAIT).build()?,
            driver: child,
            profile,
            base: format!("http://127.0.0.1:{port}"),
        };
        until("ChromeDriver startup", || {
            Ok(me.get("status").ok().filter(|v| v["ready"] == true))
        })?;
        let mut options = json!({"args":["--headless=new","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--window-size=1280,900"]});
        if let Some(dir) = &me.profile {
            let args = options["args"].as_array_mut().context("Chrome arguments")?;
            args.push(json!(format!("--user-data-dir={}", dir.path().display())));
            args.push(json!("--remote-debugging-pipe"));
            // macOS rejects a second Seatbelt profile. Chrome and every child already
            // inherit the evaluator's filesystem restrictions in this launch mode.
            if cfg!(target_os = "macos")
                && env::var("SWARM_EVAL_SANDBOX").as_deref() == Ok("macos-seatbelt")
            {
                args.push(json!("--no-sandbox"));
            }
        }
        if let Some(chrome) = chrome {
            options["binary"] = json!(chrome);
        }
        let response=me.post("session",json!({"capabilities":{"alwaysMatch":{"browserName":"chrome","goog:chromeOptions":options,"goog:loggingPrefs":{"performance":"ALL","browser":"ALL"}}}}))?;
        let id = response["sessionId"]
            .as_str()
            .context("ChromeDriver omitted session ID")?;
        me.base = format!("{}/session/{id}", me.base);
        if mobile {
            me.cdp(
                "Emulation.setDeviceMetricsOverride",
                json!({"width":412,"height":839,"deviceScaleFactor":2.625,"mobile":true}),
            )?;
            me.cdp(
                "Emulation.setTouchEmulationEnabled",
                json!({"enabled":true}),
            )?;
        }
        Ok(me)
    }
    fn request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut r = self
            .client
            .request(method, format!("{}/{}", self.base, path));
        if let Some(v) = body {
            r = r.json(&v);
        }
        let response = r.send()?;
        let status = response.status();
        let value: Value = response.json()?;
        if value["value"]["error"] == "stale element reference" {
            return Err(StaleElement.into());
        }
        ensure!(
            status.is_success() && value["value"].get("error").is_none(),
            "ChromeDriver {path}: {}",
            value["value"]["message"]
        );
        Ok(value["value"].clone())
    }
    fn get(&self, p: &str) -> Result<Value> {
        self.request(reqwest::Method::GET, p, None)
    }
    fn post(&self, p: &str, b: Value) -> Result<Value> {
        self.request(reqwest::Method::POST, p, Some(b))
    }
    pub fn goto(&self, url: &str) -> Result<()> {
        self.post("url", json!({"url":url}))?;
        Ok(())
    }
    pub fn reload(&self) -> Result<()> {
        self.post("refresh", json!({}))?;
        Ok(())
    }
    pub fn url(&self) -> Result<String> {
        Ok(self.get("url")?.as_str().context("missing URL")?.into())
    }
    pub fn cdp(&self, cmd: &str, params: Value) -> Result<Value> {
        self.post("goog/cdp/execute", json!({"cmd":cmd,"params":params}))
    }
    pub fn eval(&self, script: &str, args: Vec<Value>) -> Result<Value> {
        self.post("execute/sync", json!({"script":script,"args":args}))
    }
    pub fn xpath(&self, x: &str) -> Locator<'_> {
        Locator {
            browser: self,
            xpath: x.into(),
            role: None,
            name: None,
        }
    }
    pub fn text(&self, text: &str) -> Locator<'_> {
        let x = literal(text);
        self.xpath(&format!(
            "//*[normalize-space(.)={x} and not(*[normalize-space(.)={x}])]"
        ))
    }
    pub fn role(&self, role: &str, name: &str) -> Locator<'_> {
        let xpath = match role {
            "button" => "//button|//*[@role='button']",
            "link" => "//a[@href]",
            "heading" => "//h1|//h2|//h3|//h4|//*[@role='heading']",
            "radio" => "//input[@type='radio']",
            "checkbox" => "//input[@type='checkbox']|//*[@role='checkbox']",
            "combobox" => "//select|//*[@role='combobox']",
            "navigation" => "//nav|//*[@role='navigation']",
            "alert" => "//*[@role='alert']",
            "rowheader" => "//th|//*[@role='rowheader']",
            other => {
                return self
                    .xpath(&format!("//*[@role={}]", literal(other)))
                    .accessible(other, name);
            }
        };
        self.xpath(xpath).accessible(role, name)
    }
    pub fn label(&self, name: &str) -> Locator<'_> {
        let n = literal(name);
        self.xpath(&format!("//*[@aria-label={n}]|//input[@id=//label[normalize-space(.)={n}]/@for]|//label[normalize-space(text()[1])={n}]//*[self::select or self::textarea or self::input]|//textarea[@id=//label[normalize-space(.)={n}]/@for]"))
    }
    pub fn key(&self, value: &str) -> Result<()> {
        self.post("actions",json!({"actions":[{"type":"key","id":"keyboard","actions":[{"type":"keyDown","value":value},{"type":"keyUp","value":value}]}]}))?;
        Ok(())
    }
    pub fn windows(&self) -> Result<Vec<String>> {
        Ok(serde_json::from_value(self.get("window/handles")?)?)
    }
    pub fn switch(&self, handle: &str) -> Result<()> {
        self.post("window", json!({"handle":handle}))?;
        Ok(())
    }
    pub fn close_window(&self) -> Result<()> {
        self.request(reqwest::Method::DELETE, "window", None)?;
        Ok(())
    }
    pub fn screenshot(&self, path: &Path) -> Result<()> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        fs::write(
            path,
            STANDARD.decode(
                self.get("screenshot")?
                    .as_str()
                    .context("invalid screenshot")?,
            )?,
        )?;
        Ok(())
    }
    pub fn trace(&self, path: &Path) -> Result<()> {
        fs::write(
            path,
            serde_json::to_vec(&self.post("se/log", json!({"type":"performance"}))?)?,
        )?;
        Ok(())
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        if self.base.contains("/session/") {
            let _ = self.client.delete(&self.base).send();
        }
        let _ = self.driver.kill();
        let _ = self.driver.wait();
    }
}
pub fn literal(v: &str) -> String {
    if !v.contains('\'') {
        return format!("'{v}'");
    }
    if !v.contains('"') {
        return format!("\"{v}\"");
    }
    format!(
        "concat({})",
        v.split('\'')
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",\"'\",")
    )
}
pub struct Locator<'a> {
    browser: &'a Browser,
    xpath: String,
    role: Option<String>,
    name: Option<String>,
}
impl<'a> Locator<'a> {
    fn accessible(mut self, role: &str, name: &str) -> Self {
        self.role = Some(role.into());
        self.name = (!name.is_empty()).then(|| name.into());
        self
    }
    pub fn xpath(&self, relative: &str) -> Self {
        self.browser.xpath(&format!(
            "({}){}",
            self.xpath,
            relative.trim_start_matches('.')
        ))
    }
    fn ids(&self) -> Result<Vec<String>> {
        let nodes = self
            .browser
            .post("elements", json!({"using":"xpath","value":self.xpath}))?;
        let mut ids = Vec::new();
        for node in nodes.as_array().context("invalid element list")? {
            let id = node[ELEMENT].as_str().context("missing element ID")?;
            if let Some(role) = &self.role {
                if self.browser.get(&format!("element/{id}/computedrole"))? != role.as_str() {
                    continue;
                }
                if let Some(name) = &self.name
                    && self.browser.get(&format!("element/{id}/computedlabel"))? != name.as_str()
                {
                    continue;
                }
            }
            ids.push(id.to_owned());
        }
        Ok(ids)
    }
    fn probe<T>(&self, desc: &str, mut check: impl FnMut(&str) -> Result<Option<T>>) -> Result<T> {
        until(&format!("{desc} {} {:?}", self.xpath, self.name), || {
            for id in self.ids()? {
                if self.browser.get(&format!("element/{id}/displayed"))? == true
                    && let Some(v) = check(&id)?
                {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        })
    }
    fn element(&self) -> Result<String> {
        self.probe("visible", |id| Ok(Some(id.to_owned())))
    }
    fn obj(id: &str) -> Value {
        json!({(ELEMENT):id})
    }
    pub fn visible(&self) -> Result<()> {
        self.element()?;
        Ok(())
    }
    pub fn absent(&self) -> Result<()> {
        until(&format!("absent {}", self.xpath), || {
            Ok(self.ids()?.is_empty().then_some(()))
        })
    }
    pub fn click(&self) -> Result<()> {
        self.probe("clickable", |id| {
            if self.browser.get(&format!("element/{id}/enabled"))? != true {
                return Ok(None);
            }
            // A docked chat intentionally overlays the page. Scroll the target
            // into the free part of the viewport before a real pointer click.
            self.browser.eval(r#"
                const el = arguments[0], box = el.getBoundingClientRect();
                const x = Math.min(innerWidth - 1, Math.max(0, box.left + box.width / 2));
                const y = Math.min(innerHeight - 1, Math.max(0, box.top + box.height / 2));
                const cover = document.elementFromPoint(x,y)?.closest('.coordinator-chat');
                if (cover && !cover.contains(el)) el.scrollIntoView({block:'center',inline:'nearest'});
                return null;
            "#, vec![Self::obj(id)])?;
            self.browser
                .post(&format!("element/{id}/click"), json!({}))?;
            Ok(Some(()))
        })
    }
    pub fn fill(&self, value: &str) -> Result<()> {
        let modifier = if cfg!(target_os = "macos") {
            '\u{e03d}'
        } else {
            '\u{e009}'
        };
        // Sending keys updates controlled fields even when the new value is
        // empty. A render between lookup and typing can replace the input.
        let keys = format!("{modifier}a\u{e000}\u{e003}{value}");
        for attempt in 0..3 {
            let id = self.element()?;
            match self
                .browser
                .post(&format!("element/{id}/value"), json!({"text":keys}))
            {
                Ok(_) => return Ok(()),
                Err(e) if e.is::<StaleElement>() && attempt < 2 => continue,
                Err(e) => return Err(e),
            }
        }
        Err(StaleElement.into())
    }
    pub fn select(&self, value: &str) -> Result<()> {
        let id = self.element()?;
        let n = literal(value);
        let elem=self.browser.post(&format!("element/{id}/element"),json!({"using":"xpath","value":format!(".//option[@value={n} or normalize-space(.)={n}]")}))?;
        self.browser.post(
            &format!(
                "element/{}/click",
                elem[ELEMENT].as_str().context("missing option")?
            ),
            json!({}),
        )?;
        Ok(())
    }
    pub fn checked(&self, want: bool) -> Result<()> {
        let id = self.element()?;
        let selected = match self.attr("aria-checked")?.as_str() {
            Some(state) => state == "true",
            None => self.browser.get(&format!("element/{id}/selected"))? == true,
        };
        if selected != want {
            self.click()?;
        }
        Ok(())
    }
    pub fn focus(&self) -> Result<()> {
        let id = self.element()?;
        self.browser
            .eval("arguments[0].focus()", vec![Self::obj(&id)])?;
        Ok(())
    }
    pub fn focused(&self) -> Result<()> {
        self.probe("focused", |id| {
            Ok((self.browser.get("element/active")?[ELEMENT] == id).then_some(()))
        })
    }
    pub fn enabled(&self, want: bool) -> Result<()> {
        self.probe("enabled state", |id| {
            Ok((self.browser.get(&format!("element/{id}/enabled"))? == want).then_some(()))
        })
    }
    pub fn attr(&self, k: &str) -> Result<Value> {
        self.browser
            .get(&format!("element/{}/attribute/{k}", self.element()?))
    }
    pub fn property(&self, k: &str) -> Result<Value> {
        self.browser
            .get(&format!("element/{}/property/{k}", self.element()?))
    }
    pub fn value(&self, want: &str) -> Result<()> {
        self.probe("value", |id| {
            Ok((self.browser.get(&format!("element/{id}/property/value"))? == want).then_some(()))
        })
    }
    pub fn text_content(&self) -> Result<String> {
        Ok(self
            .browser
            .get(&format!("element/{}/text", self.element()?))?
            .as_str()
            .context("invalid text")?
            .into())
    }
    pub fn contains(&self, text: &str) -> Result<()> {
        self.probe(text, |id| {
            Ok(self
                .browser
                .get(&format!("element/{id}/text"))?
                .as_str()
                .unwrap_or_default()
                .contains(text)
                .then_some(()))
        })
    }
    pub fn rect(&self) -> Result<Value> {
        self.browser
            .get(&format!("element/{}/rect", self.element()?))
    }
    pub fn hover(&self) -> Result<()> {
        let id = self.element()?;
        self.browser.post(
            "actions",
            json!({"actions":[{
                "type":"pointer","id":"mouse","parameters":{"pointerType":"mouse"},
                "actions":[{"type":"pointerMove","origin":Self::obj(&id),"x":0,"y":0}]
            }]}),
        )?;
        Ok(())
    }
    pub fn tap(&self) -> Result<()> {
        let id = self.element()?;
        self.browser.post("actions",json!({"actions":[{"type":"pointer","id":"finger","parameters":{"pointerType":"touch"},"actions":[{"type":"pointerMove","duration":50,"origin":Self::obj(&id),"x":0,"y":0},{"type":"pointerDown","button":0},{"type":"pointerUp","button":0}]}]}))?;
        Ok(())
    }
}

//! Hidden `--selftest` mode: verifies the desktop backend end-to-end on a real window.
//! Output goes to %LOCALAPPDATA%\AutoSpaces\selftest.txt and to the parent console if any.

use crate::desktops::{DesktopBackend, Guid, InternalBackend};
use std::fs::File;
use std::io::Write;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PeekMessageW,
    TranslateMessage, GW_OWNER, MSG, PM_REMOVE,
};

struct Out {
    file: Option<File>,
    console: Option<Box<dyn Write>>,
}

impl Out {
    fn open() -> Out {
        let file = std::env::var_os("LOCALAPPDATA").and_then(|base| {
            let dir = std::path::Path::new(&base).join("AutoSpaces");
            std::fs::create_dir_all(&dir).ok()?;
            File::create(dir.join("selftest.txt")).ok()
        });
        let console: Option<Box<dyn Write>> = unsafe {
            let h = GetStdHandle(STD_OUTPUT_HANDLE);
            if !h.is_null() && h as isize != -1 {
                Some(Box::new(std::io::stdout())) // redirected by the parent
            } else if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
                std::fs::OpenOptions::new().write(true).open("CONOUT$").ok().map(|f| Box::new(f) as Box<dyn Write>)
            } else {
                None
            }
        };
        Out { file, console }
    }

    fn say(&mut self, line: &str) {
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
        if let Some(c) = self.console.as_mut() {
            let _ = writeln!(c, "{line}");
            let _ = c.flush();
        }
    }
}

/// Sleeps while keeping the STA message queue serviced.
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while Instant::now() < end {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn top_window_of(pid: u32) -> HWND {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let s = &mut *(lp as *mut (u32, HWND));
        let mut p = 0u32;
        GetWindowThreadProcessId(h, &mut p);
        if p == s.0 && IsWindowVisible(h) != 0 && GetWindow(h, GW_OWNER).is_null() {
            s.1 = h;
            return 0;
        }
        1
    }
    let mut s: (u32, HWND) = (pid, std::ptr::null_mut());
    unsafe { EnumWindows(Some(cb), &mut s as *mut _ as LPARAM) };
    s.1
}

fn wait_window(child: &Child) -> Option<HWND> {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        let h = top_window_of(child.id());
        if !h.is_null() {
            return Some(h);
        }
        pump(50);
    }
    None
}

/// Polls the public API briefly (it may lag right after a move).
fn window_on(b: &InternalBackend, hwnd: HWND, want: Guid) -> Option<Guid> {
    let mut last = None;
    for _ in 0..20 {
        last = b.window_desktop(hwnd);
        if last == Some(want) {
            break;
        }
        pump(50);
    }
    last
}

/// Milliseconds with two decimals, without pulling in float formatting.
fn ms(d: Duration) -> String {
    let us = d.as_micros();
    format!("{}.{:02}", us / 1000, (us % 1000) / 10)
}

fn short(g: Option<Guid>) -> String {
    g.map_or("none".into(), |g| g.to_string()[..8].to_string())
}

struct Runner<'a> {
    out: &'a mut Out,
    ok: bool,
}

impl Runner<'_> {
    /// Times `call`, then checks its result with `verify`. Returns the value on PASS.
    fn step<T>(
        &mut self,
        name: &str,
        call: impl FnOnce() -> crate::desktops::backend::Result<T>,
        verify: impl FnOnce(&T) -> Result<String, String>,
    ) -> Option<T> {
        let t = Instant::now();
        let r = call();
        let took = ms(t.elapsed());
        let (pass, detail, val) = match r {
            Ok(v) => match verify(&v) {
                Ok(d) => (true, d, Some(v)),
                Err(d) => (false, d, Some(v)),
            },
            Err(e) => (false, format!("error {e}"), None),
        };
        self.out.say(&format!(
            "{} {took:>8} ms  {name}: {detail}",
            if pass { "PASS" } else { "FAIL" },
        ));
        self.ok &= pass;
        if pass {
            val
        } else {
            None
        }
    }
}

fn scenario(r: &mut Runner, b: &InternalBackend, hwnd: HWND, origin: Guid, before: usize, created: &mut Option<Guid>) {
    if r.step("can_move", || b.can_move(hwnd), |&c| if c { Ok("true".into()) } else { Err("false".into()) }).is_none() {
        return;
    }
    let Some(new) = r.step("create", || b.create(), |g| Ok(format!("new desktop {g}"))) else {
        return;
    };
    *created = Some(new);
    let n = b.desktops().map(|l| l.len()).unwrap_or(0);
    if n != before + 1 {
        r.out.say(&format!("FAIL           desktop count after create: {n}, expected {}", before + 1));
        r.ok = false;
        return;
    }
    if r.step("move_window", || b.move_window(hwnd, new), |_| Ok("ok".into())).is_none() {
        return;
    }
    let on = window_on(b, hwnd, new);
    if r.step("window_desktop == new", || Ok(on), |&g| if g == Some(new) { Ok(short(g)) } else { Err(short(g)) }).is_none() {
        return;
    }
    if r.step("switch(new, false)", || b.switch(new, false), |_| match b.current() {
        Ok(c) if c == new => Ok("current == new".into()),
        c => Err(format!("current {:?}", c)),
    })
    .is_none()
    {
        return;
    }
    pump(300);
    if r.step("remove(new, origin)", || b.remove(new, origin), |_| Ok("ok".into())).is_none() {
        return;
    }
    *created = None;
    pump(200);
    r.step("current == origin", || b.current(), |&c| if c == origin { Ok(short(Some(c))) } else { Err(short(Some(c))) });
    let on = window_on(b, hwnd, origin);
    r.step("window on origin", || Ok(on), |&g| if g == Some(origin) { Ok(short(g)) } else { Err(short(g)) });
    r.step("desktop count restored", || b.desktops(), |l| {
        if l.len() == before { Ok(l.len().to_string()) } else { Err(format!("{} (expected {before})", l.len())) }
    });
}

pub fn run() -> i32 {
    let mut out = Out::open();
    out.say("AutoSpaces self-test");
    let t0 = Instant::now();
    let mut diag = Vec::new();
    let sel = InternalBackend::select(&mut diag);
    let sel_ms = ms(t0.elapsed());
    for d in &diag {
        out.say(d);
    }
    let b = match sel {
        Ok(b) => b,
        Err(u) => {
            out.say(&format!("FAIL {sel_ms:>8} ms  select: {u}"));
            return 1;
        }
    };
    out.say(&format!("PASS {sel_ms:>8} ms  select: {}", b.name()));

    let (origin, before) = match (b.current(), b.desktops()) {
        (Ok(c), Ok(l)) => (c, l.len()),
        (c, l) => {
            out.say(&format!("FAIL initial state: current {c:?}, desktops {:?}", l.map(|l| l.len())));
            return 1;
        }
    };
    out.say(&format!("origin {origin}, {before} desktop(s)"));

    let mut child = match Command::new("charmap.exe").spawn() {
        Ok(c) => c,
        Err(e) => {
            out.say(&format!("FAIL spawn charmap.exe: {e}"));
            return 1;
        }
    };
    let mut r = Runner { out: &mut out, ok: true };
    let mut created = None;
    match wait_window(&child) {
        Some(hwnd) => {
            r.out.say(&format!("charmap window {:#x}", hwnd as usize));
            pump(400);
            scenario(&mut r, &b, hwnd, origin, before, &mut created);
        }
        None => {
            r.out.say("FAIL charmap window did not appear");
            r.ok = false;
        }
    }
    let ok = r.ok;

    // Cleanup: always restore the original state.
    let _ = child.kill();
    let _ = child.wait();
    if let Some(g) = created {
        if b.current().ok() != Some(origin) {
            let _ = b.switch(origin, false);
        }
        let res = b.remove(g, origin);
        out.say(&format!("cleanup: removed leftover desktop {g}: {res:?}"));
    }
    if b.current().ok() != Some(origin) {
        let res = b.switch(origin, false);
        out.say(&format!("cleanup: switched back to origin: {res:?}"));
    }
    let n = b.desktops().map(|l| l.len()).unwrap_or(0);
    out.say(&format!(
        "final: current {}, {n} desktop(s), total {} ms",
        short(b.current().ok()),
        t0.elapsed().as_millis()
    ));
    out.say(if ok { "RESULT: PASS" } else { "RESULT: FAIL" });
    i32::from(!ok)
}

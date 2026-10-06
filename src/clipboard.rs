//! The OS clipboard, behind a trait so tests never touch the real one.

use std::fmt::Debug;

use anyhow::Result;

pub trait Clipboard: Debug {
    fn get(&mut self) -> Result<String>;
    fn set(&mut self, text: &str) -> Result<()>;
}

/// The system clipboard through arboard. The handle is opened on first use rather
/// than at startup: on a machine with no display (CI, SSH) opening fails, and that
/// should only matter if someone actually copies or pastes.
#[derive(Default)]
pub struct OsClipboard {
    inner: Option<arboard::Clipboard>,
}

impl Debug for OsClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsClipboard")
            .field("open", &self.inner.is_some())
            .finish()
    }
}

impl OsClipboard {
    fn handle(&mut self) -> Result<&mut arboard::Clipboard> {
        let handle = match self.inner.take() {
            Some(handle) => handle,
            None => arboard::Clipboard::new()?,
        };
        Ok(self.inner.insert(handle))
    }
}

impl Clipboard for OsClipboard {
    fn get(&mut self) -> Result<String> {
        Ok(self.handle()?.get_text()?)
    }

    fn set(&mut self, text: &str) -> Result<()> {
        Ok(self.handle()?.set_text(text)?)
    }
}

impl Default for Box<dyn Clipboard> {
    fn default() -> Self {
        Box::new(OsClipboard::default())
    }
}

/// An in-memory clipboard for tests.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct FakeClipboard {
    pub text: std::rc::Rc<std::cell::RefCell<String>>,
}

#[cfg(test)]
impl Clipboard for FakeClipboard {
    fn get(&mut self) -> Result<String> {
        Ok(self.text.borrow().clone())
    }

    fn set(&mut self, text: &str) -> Result<()> {
        *self.text.borrow_mut() = text.to_string();
        Ok(())
    }
}

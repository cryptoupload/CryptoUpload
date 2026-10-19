// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

pub const ARG_SEP: char = '\u{1f}';

pub const LIST_SEP: char = '\u{1e}';

#[must_use]
pub fn wire_list(msgs: &[Msg]) -> String {
    msgs.iter()
        .map(Msg::wire)
        .collect::<Vec<_>>()
        .join(&LIST_SEP.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    code: &'static str,
    args: Vec<String>,
}

impl Msg {
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            args: Vec::new(),
        }
    }

    #[must_use]
    pub fn arg(mut self, value: impl fmt::Display) -> Self {
        self.args.push(value.to_string());
        self
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn wire(&self) -> String {
        if self.args.is_empty() {
            return self.code.to_string();
        }
        let mut out = String::with_capacity(self.code.len() + 1);
        out.push_str(self.code);
        out.push(':');
        for (i, arg) in self.args.iter().enumerate() {
            if i > 0 {
                out.push(ARG_SEP);
            }
            out.push_str(arg);
        }
        out
    }
}

impl fmt::Display for Msg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)?;
        if !self.args.is_empty() {
            write!(f, " ({})", self.args.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for Msg {}

pub trait Coded {
    fn msg(&self) -> Msg;
}

impl Coded for std::io::Error {
    fn msg(&self) -> Msg {
        Msg::new("IO").arg(self)
    }
}

#[macro_export]
macro_rules! display_as_msg {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ::std::fmt::Display for $ty {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    ::std::fmt::Display::fmt(&$crate::Coded::msg(self), f)
                }
            }
        )+
    };
}

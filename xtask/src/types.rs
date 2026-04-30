use std::{error::Error, fmt::Display, str::FromStr};

#[non_exhaustive]
#[derive(Copy, Clone)]
pub enum OpenWrtArch {
    Mipsel24kc,
}

impl OpenWrtArch {
    pub fn rust_target(&self) -> &'static str {
        match self {
            OpenWrtArch::Mipsel24kc => "mipsel-unknown-linux-musl",
        }
    }
}

impl Display for OpenWrtArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenWrtArch::Mipsel24kc => f.write_str("mipsel_24kc"),
        }
    }
}

#[derive(Debug)]
pub struct UnsupportedArch(String);

impl Error for UnsupportedArch {}

impl Display for UnsupportedArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Architecture not supported: {}", self.0)
    }
}

impl FromStr for OpenWrtArch {
    type Err = UnsupportedArch;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mipsel_24kc" => Ok(Self::Mipsel24kc),
            _ => Err(UnsupportedArch(s.to_owned())),
        }
    }
}

pub enum Profile {
    Debug,
    Release,
    Custom(String),
}

impl Profile {
    pub fn target_subdir(&self) -> &str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
            Profile::Custom(custom) => custom,
        }
    }

    pub fn cargo_arg(&self) -> Box<[&str]> {
        match self {
            Profile::Debug => [].into(),
            Profile::Release => ["--release"].into(),
            Profile::Custom(custom) => ["--profile", custom].into(),
        }
    }
}

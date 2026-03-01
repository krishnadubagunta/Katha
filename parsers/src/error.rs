#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserError {
    FileDoesNotExist,
    UnreadableFile,
    UndefinedParser,
    InvalidContent,
}

impl ParserError {
    pub const fn code(self) -> i32 {
        match self {
            Self::FileDoesNotExist => 3006,
            Self::UnreadableFile => 4005,
            Self::UndefinedParser => 4006,
            Self::InvalidContent => 4007,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::FileDoesNotExist => "File does not exist",
            Self::UnreadableFile => "Unreadable file",
            Self::UndefinedParser => "Undefined parser error",
            Self::InvalidContent => "Invalid content",
        }
    }
}

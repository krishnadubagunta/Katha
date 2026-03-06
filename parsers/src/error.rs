#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserError {
    FileDoesNotExist,
    UnreadableFile,
    UndefinedParser,
    InvalidContent,
}

impl ParserError {
    /// Process exit code mapped from a parser error.
    ///
    /// # Examples
    ///
    /// ```
    /// use katha_parsers::error::ParserError;
    ///
    /// assert_eq!(ParserError::FileDoesNotExist.code(), 3006);
    /// assert_eq!(ParserError::UndefinedParser.code(), 4006);
    /// ```
    pub const fn code(self) -> i32 {
        match self {
            Self::FileDoesNotExist => 3006,
            Self::UnreadableFile => 4005,
            Self::UndefinedParser => 4006,
            Self::InvalidContent => 4007,
        }
    }

    /// Human-readable error text for display in CLI output.
    ///
    /// # Examples
    ///
    /// ```
    /// use katha_parsers::error::ParserError;
    ///
    /// assert_eq!(ParserError::UnreadableFile.message(), "Unreadable file");
    /// assert_eq!(ParserError::InvalidContent.message(), "Invalid content");
    /// ```
    pub const fn message(self) -> &'static str {
        match self {
            Self::FileDoesNotExist => "File does not exist",
            Self::UnreadableFile => "Unreadable file",
            Self::UndefinedParser => "Undefined parser error",
            Self::InvalidContent => "Invalid content",
        }
    }
}

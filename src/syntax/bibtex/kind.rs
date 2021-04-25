#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
#[repr(u16)]
pub enum SyntaxKind {
    ERROR = 0,
    MISSING,

    WHITESPACE,
    PREAMBLE_TYPE,
    STRING_TYPE,
    COMMENT_TYPE,
    ENTRY_TYPE,
    WORD,
    L_CURLY,
    R_CURLY,
    L_PAREN,
    R_PAREN,
    COMMA,
    HASH,
    QUOTE,
    EQUALITY_SIGN,
    NUMBER,
    COMMAND_NAME,

    JUNK,
    PREAMBLE,
    STRING,
    COMMENT,
    ENTRY,
    FIELD,
    VALUE,
    TOKEN,
    BRACE_GROUP,
    QUOTE_GROUP,
    ROOT,
}

impl From<SyntaxKind> for cstree::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

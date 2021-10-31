use cstree::{TextRange, TextSize};
use lsp_types::{
    CompletionParams, DocumentHighlightParams, GotoDefinitionParams, HoverParams, Position,
    ReferenceParams, RenameParams, TextDocumentPositionParams,
};
use rustc_hash::FxHashSet;

use crate::{
    syntax::{bibtex, latex, CstNode},
    DocumentData, LineIndexExt,
};

use super::FeatureRequest;

#[derive(Debug)]
pub enum Cursor {
    Latex(latex::SyntaxToken),
    Bibtex(bibtex::SyntaxToken),
    Nothing,
}

impl Cursor {
    pub fn new_latex(
        left: Option<latex::SyntaxToken>,
        right: Option<latex::SyntaxToken>,
    ) -> Option<Self> {
        let left = left?;
        let right = right?;
        let is_left_verbatim =
            is_child_of_verbatim_environment(latex::SyntaxElementRef::Token(&left));
        let is_right_verbatim =
            is_child_of_verbatim_environment(latex::SyntaxElementRef::Token(&right));

        if !is_left_verbatim && left.kind().is_command_name() {
            return Some(Self::Latex(left));
        }

        if !is_right_verbatim && right.kind() == latex::WORD {
            return Some(Self::Latex(right));
        }

        if !is_left_verbatim && left.kind() == latex::WORD {
            return Some(Self::Latex(left));
        }

        if !is_right_verbatim && right.kind().is_command_name() {
            return Some(Self::Latex(right));
        }

        if !is_left_verbatim
            && left.kind() == latex::WHITESPACE
            && left.parent().kind() == latex::KEY
        {
            return Some(Self::Latex(left));
        }

        if !is_right_verbatim
            && right.kind() == latex::WHITESPACE
            && right.parent().kind() == latex::KEY
        {
            return Some(Self::Latex(right));
        }

        Some(Self::Latex(right)).filter(|_| !is_right_verbatim)
    }

    pub fn new_bibtex(
        left: Option<bibtex::SyntaxToken>,
        right: Option<bibtex::SyntaxToken>,
    ) -> Option<Self> {
        let left = left?;
        let right = right?;

        if right.kind().is_type() {
            return Some(Self::Bibtex(right));
        }

        if left.kind().is_type() {
            return Some(Self::Bibtex(left));
        }

        if left.kind() == bibtex::COMMAND_NAME {
            return Some(Self::Bibtex(left));
        }

        if right.kind() == bibtex::WORD {
            return Some(Self::Bibtex(right));
        }

        if left.kind() == bibtex::WORD {
            return Some(Self::Bibtex(left));
        }

        if right.kind() == bibtex::COMMAND_NAME {
            return Some(Self::Bibtex(right));
        }

        Some(Self::Bibtex(right))
    }

    pub fn as_latex(&self) -> Option<&latex::SyntaxToken> {
        if let Self::Latex(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_bibtex(&self) -> Option<&bibtex::SyntaxToken> {
        if let Self::Bibtex(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn command_range(&self, offset: TextSize) -> Option<TextRange> {
        self.as_latex()
            .filter(|token| token.kind().is_command_name())
            .filter(|token| token.text_range().start() != offset)
            .map(|token| token.text_range())
            .map(|range| TextRange::new(range.start() + TextSize::from(1), range.end()))
            .or_else(|| {
                self.as_bibtex()
                    .filter(|token| token.kind() == bibtex::COMMAND_NAME)
                    .filter(|token| token.text_range().start() != offset)
                    .map(|token| token.text_range())
                    .map(|range| TextRange::new(range.start() + TextSize::from(1), range.end()))
            })
    }
}

static VERBATIM_ENVIRONMENTS: &[&str] = &[
    "asy",
    "asycode",
    "luacode",
    "lstlisting",
    "minted",
    "verbatim",
];

pub fn is_child_of_verbatim_environment(elem: latex::SyntaxElementRef) -> bool {
    let mut nodes = FxHashSet::default();

    elem.ancestors().any(|parent| {
        nodes.insert(parent.syntax().green());
        latex::Environment::cast(parent)
            .filter(|env| {
                env.begin()
                    .and_then(|begin| begin.name())
                    .and_then(|name| name.key())
                    .filter(|key| VERBATIM_ENVIRONMENTS.contains(&key.to_string().as_str()))
                    .is_some()
                    && env
                        .begin()
                        .map(|begin| !nodes.contains(&begin.syntax().green()))
                        .unwrap_or(true)
                    && env
                        .end()
                        .map(|end| !nodes.contains(&end.syntax().green()))
                        .unwrap_or(true)
            })
            .is_some()
    })
}

pub struct CursorContext<P> {
    pub request: FeatureRequest<P>,
    pub cursor: Cursor,
    pub offset: TextSize,
}

impl<P: HasPosition> CursorContext<P> {
    pub fn new(request: FeatureRequest<P>) -> Self {
        let main_document = request.main_document();
        let offset = main_document
            .line_index
            .offset_lsp(request.params.position());

        let cursor = match &main_document.data {
            DocumentData::Latex(data) => {
                let left = data.root.token_at_offset(offset).left_biased();
                let right = data.root.token_at_offset(offset).right_biased();
                Cursor::new_latex(left, right)
            }
            DocumentData::Bibtex(data) => {
                let left = data.root.token_at_offset(offset).left_biased();
                let right = data.root.token_at_offset(offset).right_biased();
                Cursor::new_bibtex(left, right)
            }
            DocumentData::BuildLog(_) => None,
        };

        Self {
            request,
            cursor: cursor.unwrap_or(Cursor::Nothing),
            offset,
        }
    }

    pub fn is_inside_latex_curly<'a>(&self, group: &impl latex::HasCurly<'a>) -> bool {
        group.small_range().contains(self.offset) || group.right_curly().is_none()
    }

    pub fn find_citation_key_word(&self) -> Option<(String, TextRange)> {
        let word = self
            .cursor
            .as_latex()
            .filter(|token| token.kind() == latex::WORD)?;

        let key = latex::Key::cast(word.parent())?;

        let group = latex::CurlyGroupWordList::cast(key.syntax().parent()?)?;
        latex::Citation::cast(group.syntax().parent()?)?;
        Some((key.to_string(), key.small_range()))
    }

    pub fn find_citation_key_command(&self) -> Option<(String, TextRange)> {
        let command = self.cursor.as_latex()?;

        let citation = latex::Citation::cast(command.parent())?;
        let key = citation.key_list()?.keys().next()?;
        Some((key.to_string(), key.small_range()))
    }

    pub fn find_entry_key(&self) -> Option<(String, TextRange)> {
        let word = self
            .cursor
            .as_bibtex()
            .filter(|token| token.kind() == bibtex::WORD)?;

        let key = bibtex::Key::cast(word.parent())?;

        bibtex::Entry::cast(key.syntax().parent()?)?;
        Some((key.to_string(), key.small_range()))
    }

    pub fn find_label_name_key(&self) -> Option<(String, TextRange)> {
        let name = self
            .cursor
            .as_latex()
            .filter(|token| token.kind() == latex::WORD)?;

        let key = latex::Key::cast(name.parent())?;

        if matches!(
            key.syntax().parent()?.parent()?.kind(),
            latex::LABEL_DEFINITION | latex::LABEL_REFERENCE | latex::LABEL_REFERENCE_RANGE
        ) {
            Some((key.to_string(), key.small_range()))
        } else {
            None
        }
    }

    pub fn find_label_name_command(&self) -> Option<(String, TextRange)> {
        let node = self.cursor.as_latex()?.parent();
        if let Some(label) = latex::LabelDefinition::cast(node) {
            let name = label.name()?.key()?;
            Some((name.to_string(), name.small_range()))
        } else if let Some(label) = latex::LabelReference::cast(node) {
            let name = label.name_list()?.keys().next()?;
            Some((name.to_string(), name.small_range()))
        } else if let Some(label) = latex::LabelReferenceRange::cast(node) {
            let name = label.from()?.key()?;
            Some((name.to_string(), name.small_range()))
        } else {
            None
        }
    }

    pub fn find_environment_name(&self) -> Option<(String, TextRange)> {
        let (name, range, group) = self.find_curly_group_word()?;

        if !matches!(group.syntax().parent()?.kind(), latex::BEGIN | latex::END) {
            return None;
        }

        Some((name, range))
    }

    pub fn find_curly_group_word(&self) -> Option<(String, TextRange, latex::CurlyGroupWord)> {
        let token = self.cursor.as_latex()?;
        let key = latex::Key::cast(token.parent());

        let group = key
            .as_ref()
            .and_then(|key| key.syntax().parent())
            .unwrap_or(token.parent());

        let group =
            latex::CurlyGroupWord::cast(group).filter(|group| self.is_inside_latex_curly(group))?;

        key.map(|key| (key.to_string(), key.small_range(), group))
            .or_else(|| Some((String::new(), TextRange::empty(self.offset), group)))
    }

    pub fn find_curly_group_word_list(
        &self,
    ) -> Option<(String, TextRange, latex::CurlyGroupWordList)> {
        let token = self.cursor.as_latex()?;
        let key = latex::Key::cast(token.parent());

        let group = key
            .as_ref()
            .and_then(|key| key.syntax().parent())
            .unwrap_or(token.parent());

        let group = latex::CurlyGroupWordList::cast(group)
            .filter(|group| self.is_inside_latex_curly(group))?;

        key.map(|key| (key.to_string(), key.small_range(), group))
            .or_else(|| Some((String::new(), TextRange::empty(self.offset), group)))
    }
}

pub trait HasPosition {
    fn position(&self) -> Position;
}

impl HasPosition for CompletionParams {
    fn position(&self) -> Position {
        self.text_document_position.position
    }
}

impl HasPosition for TextDocumentPositionParams {
    fn position(&self) -> Position {
        self.position
    }
}

impl HasPosition for RenameParams {
    fn position(&self) -> Position {
        self.text_document_position.position
    }
}

impl HasPosition for ReferenceParams {
    fn position(&self) -> Position {
        self.text_document_position.position
    }
}

impl HasPosition for HoverParams {
    fn position(&self) -> Position {
        self.text_document_position_params.position
    }
}

impl HasPosition for GotoDefinitionParams {
    fn position(&self) -> Position {
        self.text_document_position_params.position
    }
}

impl HasPosition for DocumentHighlightParams {
    fn position(&self) -> Position {
        self.text_document_position_params.position
    }
}

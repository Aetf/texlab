use std::str::FromStr;

use cstree::TextRange;
use lsp_types::{MarkupContent, MarkupKind};

use crate::{
    syntax::{
        latex::{self, HasCurly, HasKeyValueBody},
        CstNode,
    },
    WorkspaceSubset, LANGUAGE_DATA,
};

use self::LabelledObject::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LabelledFloatKind {
    Figure,
    Table,
    Listing,
    Algorithm,
}

impl LabelledFloatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Figure => "Figure",
            Self::Table => "Table",
            Self::Listing => "Listing",
            Self::Algorithm => "Algorithm",
        }
    }
}

impl FromStr for LabelledFloatKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "figure" | "subfigure" => Ok(Self::Figure),
            "table" | "subtable" => Ok(Self::Table),
            "listing" | "lstlisting" => Ok(Self::Listing),
            "algorithm" => Ok(Self::Algorithm),
            _ => Err(()),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum LabelledObject {
    Section {
        prefix: &'static str,
        text: String,
    },
    Float {
        kind: LabelledFloatKind,
        caption: String,
    },
    Theorem {
        kind: String,
        description: Option<String>,
    },
    Equation,
    EnumItem,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RenderedLabel {
    pub range: TextRange,
    pub number: Option<String>,
    pub object: LabelledObject,
}

impl RenderedLabel {
    pub fn reference(&self) -> String {
        match &self.number {
            Some(number) => match &self.object {
                Section { prefix, text } => format!("{} {} ({})", prefix, number, text),
                Float { kind, caption } => format!("{} {}: {}", kind.as_str(), number, caption),
                Theorem {
                    kind,
                    description: None,
                } => format!("{} {}", kind, number),
                Theorem {
                    kind,
                    description: Some(description),
                } => format!("{} {} ({})", kind, number, description),
                Equation => format!("Equation ({})", number),
                EnumItem => format!("Item {}", number),
            },
            None => match &self.object {
                Section { prefix, text } => format!("{} ({})", prefix, text),
                Float { kind, caption } => format!("{}: {}", kind.as_str(), caption),
                Theorem {
                    kind,
                    description: None,
                } => kind.into(),
                Theorem {
                    kind,
                    description: Some(description),
                } => format!("{} ({})", kind, description),
                Equation => "Equation".into(),
                EnumItem => "Item".into(),
            },
        }
    }

    pub fn detail(&self) -> Option<String> {
        match &self.object {
            Section { .. } | Theorem { .. } | Equation | EnumItem => Some(self.reference()),
            Float { kind, .. } => {
                let result = match &self.number {
                    Some(number) => format!("{} {}", kind.as_str(), number),
                    None => kind.as_str().to_owned(),
                };
                Some(result)
            }
        }
    }

    pub fn documentation(&self) -> MarkupContent {
        MarkupContent {
            kind: MarkupKind::PlainText,
            value: self.reference(),
        }
    }
}

pub fn render_label<'a>(
    subset: &'a WorkspaceSubset,
    label_name: &str,
    mut label: Option<latex::LabelDefinition<'a>>,
) -> Option<RenderedLabel> {
    let mut number = find_label_number(subset, label_name).map(ToString::to_string);

    for document in &subset.documents {
        if let Some(data) = document.data.as_latex() {
            label = label.or_else(|| find_label_definition(&data.root, label_name));
        }
    }

    label?.syntax().ancestors().find_map(|parent| {
        render_label_float(parent, &mut number)
            .or_else(|| render_label_section(parent, &mut number))
            .or_else(|| render_label_enum_item(parent, &mut number))
            .or_else(|| render_label_equation(parent, &mut number))
            .or_else(|| render_label_theorem(subset, parent, &mut number))
    })
}

pub fn find_label_definition<'a>(
    root: &'a latex::SyntaxNode,
    label_name: &str,
) -> Option<latex::LabelDefinition<'a>> {
    root.descendants()
        .filter_map(latex::LabelDefinition::cast)
        .find(|label| {
            label
                .name()
                .and_then(|name| name.word())
                .map(|name| name.text())
                == Some(label_name)
        })
}

pub fn find_label_number<'a>(subset: &'a WorkspaceSubset, label_name: &str) -> Option<&'a str> {
    subset.documents.iter().find_map(|document| {
        document
            .data
            .as_latex()
            .and_then(|data| data.extras.label_numbers_by_name.get(label_name))
            .map(|number| number.as_str())
    })
}

fn render_label_float(
    parent: &latex::SyntaxNode,
    number: &mut Option<String>,
) -> Option<RenderedLabel> {
    let environment = latex::Environment::cast(parent)?;
    let environment_name = environment.begin()?.name()?.word()?.text();
    let kind = LabelledFloatKind::from_str(&environment_name).ok()?;
    let caption = find_caption_by_parent(&parent)?;
    Some(RenderedLabel {
        range: environment.small_range(),
        number: number.take(),
        object: LabelledObject::Float { caption, kind },
    })
}

fn render_label_section(
    parent: &latex::SyntaxNode,
    number: &mut Option<String>,
) -> Option<RenderedLabel> {
    let section = latex::Section::cast(parent)?;
    let text_group = section.name()?;
    let text = text_group.content_text()?;

    Some(RenderedLabel {
        range: section.small_range(),
        number: number.take(),
        object: LabelledObject::Section {
            prefix: match section.syntax().kind() {
                latex::PART => "Part",
                latex::CHAPTER => "Chapter",
                latex::SECTION => "Section",
                latex::SUBSECTION => "Subsection",
                latex::SUBSUBSECTION => "Subsubsection",
                latex::PARAGRAPH => "Paragraph",
                latex::SUBPARAGRAPH => "Subparagraph",
                _ => unreachable!(),
            },
            text,
        },
    })
}

fn render_label_enum_item(
    parent: &latex::SyntaxNode,
    number: &mut Option<String>,
) -> Option<RenderedLabel> {
    let enum_item = latex::EnumItem::cast(parent)?;
    Some(RenderedLabel {
        range: enum_item.small_range(),
        number: enum_item
            .label()
            .and_then(|number| number.word())
            .map(|number| number.text().to_string())
            .or_else(|| number.take()),
        object: LabelledObject::EnumItem,
    })
}

fn render_label_equation(
    parent: &latex::SyntaxNode,
    number: &mut Option<String>,
) -> Option<RenderedLabel> {
    let environment = latex::Environment::cast(parent)?;
    let environment_name = environment.begin()?.name()?.word()?.text();

    if !LANGUAGE_DATA
        .math_environments
        .iter()
        .any(|name| name == environment_name)
    {
        return None;
    }

    Some(RenderedLabel {
        range: environment.small_range(),
        number: number.take(),
        object: LabelledObject::Equation,
    })
}

fn render_label_theorem(
    subset: &WorkspaceSubset,
    parent: &latex::SyntaxNode,
    number: &mut Option<String>,
) -> Option<RenderedLabel> {
    let environment = latex::Environment::cast(parent)?;
    let begin = environment.begin()?;
    let description = begin
        .options()
        .and_then(|options| options.body())
        .map(|body| body.syntax().text().to_string());

    let environment_name = begin.name()?.word()?.text();

    let theorem = subset.documents.iter().find_map(|document| {
        document.data.as_latex().and_then(|data| {
            data.extras
                .theorem_environments
                .iter()
                .find(|theorem| theorem.name.as_str() == environment_name)
        })
    })?;

    Some(RenderedLabel {
        range: environment.small_range(),
        number: number.take(),
        object: LabelledObject::Theorem {
            kind: theorem.description.clone(),
            description,
        },
    })
}

pub fn find_caption_by_parent(parent: &latex::SyntaxNode) -> Option<String> {
    parent
        .children()
        .filter_map(latex::Caption::cast)
        .find_map(|node| node.long())
        .and_then(|node| node.content_text())
}

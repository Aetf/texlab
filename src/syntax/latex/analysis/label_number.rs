use crate::syntax::{latex, CstNode};

use super::LatexAnalyzerContext;

pub fn analyze_label_number(
    context: &mut LatexAnalyzerContext,
    node: &latex::SyntaxNode,
) -> Option<()> {
    let number = latex::LabelNumber::cast(node)?;
    let name = number.name()?.word()?.text().into();
    let text = number
        .text()?
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_node())
        .find(|node| node.kind() == latex::TEXT || node.kind() == latex::MIXED_GROUP)?
        .text()
        .to_string();

    context.extras.label_numbers_by_name.insert(name, text);
    Some(())
}

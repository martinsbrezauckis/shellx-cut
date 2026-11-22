//! Assemble the legacy left-nested redaction union in one output pass.

pub(super) fn indicators(indicators: Vec<String>) -> String {
    let Some(first) = indicators.first() else {
        return "0".to_string();
    };
    if indicators.len() == 1 {
        return first.clone();
    }
    let capacity =
        indicators.iter().map(String::len).sum::<usize>() + (indicators.len() - 1) * "max(,)".len();
    let mut text = String::with_capacity(capacity);
    for _ in 1..indicators.len() {
        text.push_str("max(");
    }
    text.push_str(first);
    for next in indicators.iter().skip(1) {
        text.push(',');
        text.push_str(next);
        text.push(')');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::indicators;

    #[test]
    fn legacy_union_text_for_zero_one_two_three_regions() {
        assert_eq!(indicators(vec![]), "0");
        assert_eq!(indicators(vec!["a".into()]), "a");
        assert_eq!(indicators(vec!["a".into(), "b".into()]), "max(a,b)");
        assert_eq!(
            indicators(vec!["a".into(), "b".into(), "c".into()]),
            "max(max(a,b),c)"
        );
    }
}

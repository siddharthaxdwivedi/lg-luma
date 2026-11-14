//! Minimal GBNF parser for whisper.cpp command grammars.
//!
//! whisper.cpp consumes a grammar as one element list per rule, each terminated
//! by `END`, referenced through `const whisper_grammar_element **`. This module
//! parses the subset of GBNF used by `assets/commands.gbnf`: rule definitions,
//! alternation, grouping, literals, character classes, `*`, `+`, `?` and rule
//! references. It has no external dependencies so it can be tested on its own
//! with `rustc --test src/stt/grammar.rs`.

/// Grammar element kinds mirroring `whisper_gretype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    End,
    Alt,
    RuleRef,
    Char,
    CharNot,
    CharRangeUpper,
    CharAlt,
}

/// One grammar element; `value` is a codepoint or a rule index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element {
    pub kind: ElementKind,
    pub value: u32,
}

/// Parsed grammar: one element list per rule, each ending with `End`.
#[derive(Debug, Clone, Default)]
pub struct Grammar {
    pub rules: Vec<Vec<Element>>,
}

struct Parser {
    symbols: Vec<String>,
    rules: Vec<Option<Vec<Element>>>,
    src: Vec<char>,
}

impl Parser {
    fn symbol_id(&mut self, name: &str) -> u32 {
        if let Some(index) = self.symbols.iter().position(|known| known == name) {
            return index as u32;
        }
        let id = self.symbols.len() as u32;
        self.symbols.push(name.to_string());
        id
    }

    fn generated_id(&mut self, base: &str) -> u32 {
        let id = self.symbols.len() as u32;
        self.symbols.push(format!("{base}_{id}"));
        id
    }

    fn set_rule(&mut self, id: u32, rule: Vec<Element>) {
        let index = id as usize;
        if self.rules.len() <= index {
            self.rules.resize(index + 1, None);
        }
        self.rules[index] = Some(rule);
    }

    fn at(&self, pos: usize) -> char {
        self.src.get(pos).copied().unwrap_or('\0')
    }

    fn skip_space(&self, mut pos: usize, newline_ok: bool) -> usize {
        loop {
            match self.at(pos) {
                ' ' | '\t' => pos += 1,
                '#' => {
                    while self.at(pos) != '\0' && self.at(pos) != '\n' && self.at(pos) != '\r' {
                        pos += 1;
                    }
                }
                '\r' | '\n' if newline_ok => pos += 1,
                _ => return pos,
            }
        }
    }

    fn is_word(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '-'
    }

    fn name(&self, pos: usize) -> Result<(String, usize), String> {
        let mut end = pos;
        while Self::is_word(self.at(end)) {
            end += 1;
        }
        if end == pos {
            return Err(format!("expected a rule name at offset {pos}"));
        }
        Ok((self.src[pos..end].iter().collect(), end))
    }

    fn hex(&self, pos: usize, digits: usize) -> Result<(u32, usize), String> {
        let end = pos + digits;
        let mut value = 0u32;
        for index in pos..end {
            let digit = self
                .at(index)
                .to_digit(16)
                .ok_or_else(|| format!("expected {digits} hex digits at offset {pos}"))?;
            value = value * 16 + digit;
        }
        Ok((value, end))
    }

    fn character(&self, pos: usize) -> Result<(u32, usize), String> {
        match self.at(pos) {
            '\\' => match self.at(pos + 1) {
                'x' => self.hex(pos + 2, 2),
                'u' => self.hex(pos + 2, 4),
                'U' => self.hex(pos + 2, 8),
                't' => Ok(('\t' as u32, pos + 2)),
                'r' => Ok(('\r' as u32, pos + 2)),
                'n' => Ok(('\n' as u32, pos + 2)),
                '\\' => Ok(('\\' as u32, pos + 2)),
                '"' => Ok(('"' as u32, pos + 2)),
                '[' => Ok(('[' as u32, pos + 2)),
                ']' => Ok((']' as u32, pos + 2)),
                _ => Err(format!("unknown escape at offset {pos}")),
            },
            '\0' => Err("unexpected end of grammar".to_string()),
            c => Ok((c as u32, pos + 1)),
        }
    }

    fn sequence(
        &mut self,
        mut pos: usize,
        rule_name: &str,
        out: &mut Vec<Element>,
        nested: bool,
    ) -> Result<usize, String> {
        let mut last = out.len();
        loop {
            match self.at(pos) {
                '"' => {
                    pos += 1;
                    last = out.len();
                    while self.at(pos) != '"' {
                        let (value, next) = self.character(pos)?;
                        out.push(Element {
                            kind: ElementKind::Char,
                            value,
                        });
                        pos = next;
                    }
                    pos = self.skip_space(pos + 1, nested);
                }
                '[' => {
                    pos += 1;
                    let first_kind = if self.at(pos) == '^' {
                        pos += 1;
                        ElementKind::CharNot
                    } else {
                        ElementKind::Char
                    };
                    last = out.len();
                    while self.at(pos) != ']' {
                        let (value, next) = self.character(pos)?;
                        let kind = if out.len() > last {
                            ElementKind::CharAlt
                        } else {
                            first_kind
                        };
                        out.push(Element { kind, value });
                        pos = next;
                        if self.at(pos) == '-' && self.at(pos + 1) != ']' {
                            let (upper, next) = self.character(pos + 1)?;
                            out.push(Element {
                                kind: ElementKind::CharRangeUpper,
                                value: upper,
                            });
                            pos = next;
                        }
                    }
                    pos = self.skip_space(pos + 1, nested);
                }
                c if Self::is_word(c) => {
                    let (name, next) = self.name(pos)?;
                    let id = self.symbol_id(&name);
                    pos = self.skip_space(next, nested);
                    last = out.len();
                    out.push(Element {
                        kind: ElementKind::RuleRef,
                        value: id,
                    });
                }
                '(' => {
                    pos = self.skip_space(pos + 1, true);
                    let sub = self.generated_id(rule_name);
                    pos = self.alternates(pos, rule_name, sub, true)?;
                    last = out.len();
                    out.push(Element {
                        kind: ElementKind::RuleRef,
                        value: sub,
                    });
                    if self.at(pos) != ')' {
                        return Err(format!("expected ')' at offset {pos}"));
                    }
                    pos = self.skip_space(pos + 1, nested);
                }
                op @ ('*' | '+' | '?') => {
                    if last == out.len() {
                        return Err(format!("'{op}' without a preceding item at offset {pos}"));
                    }
                    let sub = self.generated_id(rule_name);
                    let mut rule: Vec<Element> = out[last..].to_vec();
                    if op == '*' || op == '+' {
                        rule.push(Element {
                            kind: ElementKind::RuleRef,
                            value: sub,
                        });
                    }
                    rule.push(Element {
                        kind: ElementKind::Alt,
                        value: 0,
                    });
                    if op == '+' {
                        rule.extend_from_slice(&out[last..]);
                    }
                    rule.push(Element {
                        kind: ElementKind::End,
                        value: 0,
                    });
                    self.set_rule(sub, rule);
                    out.truncate(last);
                    out.push(Element {
                        kind: ElementKind::RuleRef,
                        value: sub,
                    });
                    pos = self.skip_space(pos + 1, nested);
                }
                _ => return Ok(pos),
            }
        }
    }

    fn alternates(
        &mut self,
        mut pos: usize,
        rule_name: &str,
        rule_id: u32,
        nested: bool,
    ) -> Result<usize, String> {
        let mut rule = Vec::new();
        pos = self.sequence(pos, rule_name, &mut rule, nested)?;
        while self.at(pos) == '|' {
            rule.push(Element {
                kind: ElementKind::Alt,
                value: 0,
            });
            pos = self.skip_space(pos + 1, true);
            pos = self.sequence(pos, rule_name, &mut rule, nested)?;
        }
        rule.push(Element {
            kind: ElementKind::End,
            value: 0,
        });
        self.set_rule(rule_id, rule);
        Ok(pos)
    }

    fn rule(&mut self, pos: usize) -> Result<usize, String> {
        let (name, next) = self.name(pos)?;
        let mut pos = self.skip_space(next, false);
        if self.at(pos) != ':' || self.at(pos + 1) != ':' || self.at(pos + 2) != '=' {
            return Err(format!("expected '::=' at offset {pos}"));
        }
        pos = self.skip_space(pos + 3, true);
        let id = self.symbol_id(&name);
        pos = self.alternates(pos, &name, id, false)?;
        match self.at(pos) {
            '\r' => pos += if self.at(pos + 1) == '\n' { 2 } else { 1 },
            '\n' => pos += 1,
            '\0' => {}
            _ => return Err(format!("expected a newline at offset {pos}")),
        }
        Ok(self.skip_space(pos, true))
    }
}

/// Parse GBNF source into whisper.cpp grammar rules.
pub fn parse(source: &str) -> Result<Grammar, String> {
    let mut parser = Parser {
        symbols: Vec::new(),
        rules: Vec::new(),
        src: source.chars().collect(),
    };
    let mut pos = parser.skip_space(0, true);
    while parser.at(pos) != '\0' {
        pos = parser.rule(pos)?;
    }
    let mut rules = Vec::with_capacity(parser.symbols.len());
    for index in 0..parser.symbols.len() {
        match parser.rules.get(index).and_then(|rule| rule.as_ref()) {
            Some(rule) => rules.push(rule.clone()),
            None => {
                return Err(format!(
                    "rule '{}' is referenced but never defined",
                    parser.symbols[index]
                ));
            }
        }
    }
    if rules.is_empty() {
        return Err("grammar has no rules".to_string());
    }
    Ok(Grammar { rules })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(rule: &[Element]) -> Vec<ElementKind> {
        rule.iter().map(|element| element.kind).collect()
    }

    #[test]
    fn parses_literals_and_references() {
        let grammar = parse("root ::= \"open \" app\napp ::= \"youtube\"\n").unwrap();
        assert_eq!(grammar.rules.len(), 2);
        assert_eq!(grammar.rules[0][0].kind, ElementKind::Char);
        assert_eq!(grammar.rules[0][0].value, 'o' as u32);
        assert_eq!(grammar.rules[0][5].kind, ElementKind::RuleRef);
        assert_eq!(*kinds(&grammar.rules[0]).last().unwrap(), ElementKind::End);
    }

    #[test]
    fn parses_alternation_grouping_and_repetition() {
        let grammar =
            parse("root ::= \"turn \" (\"on\" | \"off\") phrase\nphrase ::= [a-z0-9 '-]+\n")
                .unwrap();
        assert!(grammar.rules.len() >= 3);
        assert!(
            grammar
                .rules
                .iter()
                .any(|rule| rule.iter().any(|element| element.kind == ElementKind::Alt))
        );
        assert!(
            grammar
                .rules
                .iter()
                .any(|rule| rule.iter().any(|e| e.kind == ElementKind::CharRangeUpper))
        );
    }

    #[test]
    fn parses_the_shipped_command_grammar() {
        let source = include_str!("../../assets/commands.gbnf");
        let grammar = parse(source).unwrap();
        assert_eq!(grammar.rules[0][0].kind, ElementKind::RuleRef);
        for rule in &grammar.rules {
            assert_eq!(rule.last().map(|e| e.kind), Some(ElementKind::End));
        }
    }

    #[test]
    fn rejects_undefined_rules() {
        assert!(parse("root ::= missing\n").is_err());
    }
}

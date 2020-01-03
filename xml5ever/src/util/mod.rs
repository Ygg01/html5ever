// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use mac::{_tt_as_expr_hack, matches};

/// Is the character an ASCII alphanumeric character?
pub fn is_ascii_alnum(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='z' | 'A'..='Z')
}

pub fn is_xml_char(c: char) -> bool {
    matches!(c, '\x09' | '\x0A' | '\x0D' | '\x20'..='\u{D7FF}'| '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

pub fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | '_' | 'a'..='z' | 'A'..='Z' | '\u{C0}'..='\u{D6}' |
        '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' |
        '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' |
        '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' |
        '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' |
        '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}'
    )
}

pub fn is_name_char(c: char) -> bool {
    matches!(c,
        ':' | '_' | '-' | '.' | '\u{B7}' |
        'a'..='z' | 'A'..='Z' | '0'..='9' |
        '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' |
        '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' |
        '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' |
        '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' |
        '\u{0300}'..='\u{036F}' | '\u{203F}'..='\u{2040}' |
        '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' |
        '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}'
    )
}

#[cfg(test)]
#[allow(non_snake_case)]
mod test {
    use super::{is_ascii_alnum, is_xml_char};
    use mac::test_eq;

    test_eq!(is_alnum_a, is_ascii_alnum('a'), true);
    test_eq!(is_alnum_A, is_ascii_alnum('A'), true);
    test_eq!(is_alnum_1, is_ascii_alnum('1'), true);
    test_eq!(is_not_alnum_symbol, is_ascii_alnum('!'), false);
    test_eq!(is_not_alnum_nonascii, is_ascii_alnum('\u{a66e}'), false);

    test_eq!(is_xml_char_a, is_xml_char('a'), true);
    test_eq!(is_xml_char_A, is_xml_char('A'), true);
    test_eq!(is_xml_char_excl, is_xml_char('!'), true);
    test_eq!(is_not_xml_char_1F, is_xml_char('\x1F'), false);
    test_eq!(is_not_xml_char_FFFE, is_xml_char('\u{FFFE}'), false);
    test_eq!(is_not_xml_char_FFFF, is_xml_char('\u{FFFF}'), false);
}

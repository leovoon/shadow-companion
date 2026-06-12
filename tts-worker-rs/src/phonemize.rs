//! Faithful port of phonemizer punctuation preserve/restore + line postprocess.
//!
//! Mirrors `phonemizer.backend.espeak.espeak.EspeakBackend._phonemize_aux`,
//! `phonemizer.punctuation.Punctuation.preserve/restore`, and
//! `BaseBackend.phonemize` (preserve → phonemize → postprocess → restore).
//!
//! See RUST_WORKER_PLAN §5.12 and Appendix A.2 for the normative spec.

use crate::espeak::Espeak;

// ---------------------------------------------------------------------------
// Mark
// ---------------------------------------------------------------------------

/// A punctuation mark extracted during `preserve_punctuation`, with its
/// positional classification (B/E/I/A) relative to the original line.
#[derive(Debug, Clone)]
pub struct Mark {
    /// The matched punctuation string (including its surrounding whitespace).
    pub mark: String,
    /// Position: 'B'egin, 'E'nd, 'I'ntermediate, 'A'lone.
    pub position: char,
}

// ---------------------------------------------------------------------------
// Default punctuation marks (must match phonemizer exactly)
// ---------------------------------------------------------------------------

const DEFAULT_MARKS: &str = ";:,.!?¡¿\u{2014}…\u{00ab}\u{00bb}\u{201c}\u{201d}(){}[]";

/// Returns true if `c` is one of the default punctuation mark characters.
fn is_mark_char(c: char) -> bool {
    DEFAULT_MARKS.contains(c)
}

// ---------------------------------------------------------------------------
// preserve_punctuation
// ---------------------------------------------------------------------------

/// Split `text` on runs of punctuation mark characters (with surrounding
/// whitespace), returning the non-mark text chunks and the extracted marks.
///
/// Port of `phonemizer.punctuation.Punctuation._preserve_line`.
///
/// The regex pattern from Python: `(\s*[MARKS]+\s*)+`
/// This matches maximal runs of: (optional-ws marks+ optional-ws)+
///
/// Algorithm (manual, no regex crate):
/// 1. Find all maximal matches of the pattern in the line.
/// 2. Classify each match as B/E/I/A based on its position in the line.
/// 3. Split the line on each match, collecting the non-mark text chunks.
///
/// Example: `"Hello, world! This is a test."`
///   → chunks: `["Hello", "world", "This is a test"]`
///   → marks:  `[Mark{", ", I}, Mark{"! ", I}, Mark{".", E}]`
pub fn preserve_punctuation(text: &str) -> (Vec<String>, Vec<Mark>) {
    let text = text.trim_end();
    if text.is_empty() {
        return (vec![], vec![]);
    }

    // Step 1: Find all mark-char positions
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();

    // Find contiguous runs of mark chars
    let mut mark_runs: Vec<(usize, usize)> = Vec::new(); // [start, end) in char indices
    let mut i = 0;
    while i < n {
        if is_mark_char(chars[i]) {
            let start = i;
            while i < n && is_mark_char(chars[i]) {
                i += 1;
            }
            mark_runs.push((start, i));
        } else {
            i += 1;
        }
    }

    if mark_runs.is_empty() {
        return (vec![text.to_string()], vec![]);
    }

    // Step 2: Extend each run to include surrounding whitespace and merge
    // The regex `(\s*[MARKS]+\s*)+` means: each group has optional ws,
    // marks+, optional ws. The outer `+` means these groups repeat.
    // So a match is: ws? marks+ ws? (ws marks+ ws?)*
    // We expand each mark_run to include leading/trailing ws, then merge
    // overlapping/adjacent runs.

    let mut expanded: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in &mark_runs {
        let mut s = start;
        let mut e = end;
        // Extend left to include whitespace
        while s > 0 && chars[s - 1] == ' ' {
            s -= 1;
        }
        // Extend right to include whitespace
        while e < n && chars[e] == ' ' {
            e += 1;
        }
        expanded.push((s, e));
    }

    // Merge overlapping/adjacent runs
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in expanded {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                // Overlapping or adjacent — merge
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    // Build match strings
    let matches: Vec<(usize, usize, String)> = merged
        .into_iter()
        .map(|(s, e)| (s, e, chars[s..e].iter().collect::<String>()))
        .collect();

    // Step 2: If no matches, return the whole text as one chunk
    if matches.is_empty() {
        return (vec![text.to_string()], vec![]);
    }

    // Special case: entire line is just marks
    if matches.len() == 1 && matches[0].0 == 0 && matches[0].1 == n {
        let (_, _, ref matched) = matches[0];
        return (vec![], vec![Mark { mark: matched.clone(), position: 'A' }]);
    }

    // Step 3: Classify positions
    let mut marks: Vec<Mark> = Vec::with_capacity(matches.len());
    for (idx, (start, end, ref matched)) in matches.iter().enumerate() {
        let position = if *start == 0 {
            'B'
        } else if *end == n {
            'E'
        } else if idx == 0 {
            // First match but not at start (shouldn't happen since we check start==0)
            'I'
        } else if idx == matches.len() - 1 {
            // Last match but not at end (shouldn't happen since we check end==n)
            'I'
        } else {
            'I'
        };
        marks.push(Mark { mark: matched.clone(), position });
    }

    // Step 4: Split the line on each mark (Python's split logic)
    // Python: for mark in marks:
    //     split = line.split(mark.mark)
    //     prefix, suffix = split[0], mark.mark.join(split[1:])
    //     preserved_line.append(prefix)
    //     line = suffix
    // return preserved_line + [line], marks
    let mut remaining = text.to_string();
    let mut chunks: Vec<String> = Vec::with_capacity(marks.len() + 1);

    for mark in &marks {
        if let Some(pos) = remaining.find(&mark.mark) {
            let prefix = remaining[..pos].to_string();
            let suffix = remaining[pos + mark.mark.len()..].to_string();
            chunks.push(prefix);
            remaining = suffix;
        } else {
            // Shouldn't happen if match detection is correct, but handle gracefully
            chunks.push(std::mem::take(&mut remaining));
        }
    }
    chunks.push(remaining);

    // Filter empty chunks (Python: `[line for line in preserved_text if line]`)
    chunks.retain(|c| !c.is_empty());

    (chunks, marks)
}

// ---------------------------------------------------------------------------
// postprocess_line
// ---------------------------------------------------------------------------

/// Post-process a single line of espeak output.
///
/// Port of `EspeakBackend._postprocess_line` with `with_stress=True`,
/// `strip=False`, `tie=None` (→ phone separator ''), `word=' '`.
///
/// Steps:
/// 1. strip(); `'\n'` → `' '`; `'  '` → `' '`
/// 2. `_+` → `_`; `'_ '` → `' '`
/// 3. language-switch remove-flags: delete all `(...)` groups
/// 4. Per word: trim, keep stress marks (with_stress=True → no stripping),
///    append `'_'` (strip=False, tie=None), then replace `'_'` with `''`,
///    then append word separator `' '`
/// 5. Result: phonemes concatenated per word, spaces between words,
///    ONE TRAILING SPACE per line.
fn postprocess_line(line: &str) -> String {
    // 1. strip + normalize whitespace
    let mut line = line.trim().to_string();
    if line.is_empty() {
        return String::new();
    }
    line = line.replace('\n', " ");
    while line.contains("  ") {
        line = line.replace("  ", " ");
    }

    // 2. espeak-ng bug #694 workaround: extra separators at word end
    //    `_+` → `_` then `'_ '` → `' '`
    // Replace runs of two or more '_' with a single '_'
    {
        let mut result = String::with_capacity(line.len());
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '_' {
                result.push('_');
                // Skip consecutive underscores
                while i < chars.len() && chars[i] == '_' {
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        line = result;
    }
    // `'_ '` → `' '`
    line = line.replace("_ ", " ");

    // 3. language-switch remove-flags: if `(some_text)` found, delete ALL such groups
    if line.contains('(') {
        let mut out = String::with_capacity(line.len());
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let mut depth = 0i32;
        while i < chars.len() {
            if chars[i] == '(' {
                depth += 1;
                i += 1;
                continue;
            }
            if chars[i] == ')' {
                if depth > 0 {
                    depth -= 1;
                }
                i += 1;
                continue;
            }
            if depth == 0 {
                out.push(chars[i]);
            }
            i += 1;
        }
        line = out;
    }

    if line.is_empty() {
        return String::new();
    }

    // 4. Per word processing
    // separator.phone = '' (tie=None → replace '_' with '')
    // separator.word = ' '
    // with_stress=True → _process_stress is identity (keep stress marks)
    // strip=False, tie=None: word += '_' then replace '_' with ''
    let mut out_line = String::new();
    for word in line.split(' ') {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        // with_stress=True: _process_stress is identity — keep the word as-is
        // strip=False, tie=None: word += '_'
        let mut processed = word.to_string();
        processed.push('_');
        // _process_tie: replace '_' with separator.phone ('') → delete all underscores
        processed = processed.replace('_', "");
        // Append word separator
        out_line.push_str(&processed);
        out_line.push(' ');
    }

    out_line
}

// ---------------------------------------------------------------------------
// restore_punctuation
// ---------------------------------------------------------------------------

/// Re-insert punctuation marks into the phonemized text.
///
/// Port of `Punctuation.restore`. The algorithm walks `text` chunks and
/// `marks` together, inserting marks at B/E/I/A positions.
///
/// Key semantics from the Python source:
/// - All marks from a single line have the same index (0).
/// - The `pos` counter tracks how many text chunks have been *emitted*
///   to `punctuated_text`.
/// - `current_mark.index == pos` gates when a mark is applied. Since all
///   marks have index=0 and pos starts at 0, all marks are processed
///   before any text chunk is emitted (except via E/A which emit and
///   advance pos).
/// - For I-marks: merge text chunks (no pos advance).
/// - For E-marks: emit the current chunk with the mark appended, advance pos.
/// - For B-marks: prepend the mark to the current chunk (no pos advance).
/// - For A-marks: emit the mark alone, advance pos.
fn restore_punctuation(chunks: Vec<String>, marks: Vec<Mark>) -> String {
    if marks.is_empty() {
        return chunks.join("");
    }

    let sep_word = " ";
    // strip = False

    let mut text: Vec<String> = chunks;
    let mut marks: Vec<Mark> = marks;
    let mut punctuated: Vec<String> = Vec::new();
    let mut pos: usize = 0;
    // All marks from a single line have index 0
    let mark_index: usize = 0;

    while !text.is_empty() || !marks.is_empty() {
        if marks.is_empty() {
            // No more marks → emit remaining text chunks
            for line in text.drain(..) {
                let mut line = line;
                // strip=False, sep.word=' ': if not strip and sep.word and not line.endswith(sep_word)
                if !line.ends_with(sep_word) {
                    line.push_str(sep_word);
                }
                punctuated.push(line);
            }
        } else if text.is_empty() {
            // No more text → emit all remaining marks joined
            let joined: String = marks.iter().map(|m| m.mark.as_str()).collect();
            let joined = joined.replace(' ', sep_word);
            punctuated.push(joined);
            marks.clear();
        } else {
            let current_mark = &marks[0];
            if current_mark.position == 'B' || current_mark.position == 'I'
                || current_mark.position == 'E' || current_mark.position == 'A'
            {
                // In the single-line case (all marks have index 0),
                // index == pos is always true at pos=0 and false after pos advances.
                // But Python processes marks eagerly: if index==pos, process immediately.
                // If index != pos, emit text[0] and advance pos.
                // Since all marks have index=0, once pos > 0, remaining marks
                // won't match index and text chunks get emitted.
                // However, in practice for the phonemizer use case, marks are always
                // processed before text is emitted (I-marks merge chunks, E-marks emit).
                //
                // The real Python logic: if current_mark.index == pos → process mark;
                // else → emit text[0], advance pos.
                if mark_index != pos {
                    // Index mismatch → emit current text chunk
                    punctuated.push(text.remove(0));
                    pos += 1;
                    continue;
                }

                let mark = marks.remove(0);
                let mark_text = mark.mark.replace(' ', sep_word);

                // Remove trailing word-separator from text[0] before attaching mark
                let trim_len = if text[0].ends_with(sep_word) {
                    sep_word.len()
                } else {
                    0
                };
                if trim_len > 0 {
                    let new_len = text[0].len() - trim_len;
                    text[0].truncate(new_len);
                }

                match mark.position {
                    'B' => {
                        text[0] = format!("{}{}", mark_text, text[0]);
                    }
                    'E' => {
                        let out = format!(
                            "{}{}{}",
                            text.remove(0),
                            mark_text,
                            if mark_text.ends_with(sep_word) {
                                ""
                            } else {
                                sep_word
                            }
                        );
                        punctuated.push(out);
                        pos += 1;
                    }
                    'A' => {
                        let out = format!(
                            "{}{}",
                            mark_text,
                            if mark_text.ends_with(sep_word) {
                                ""
                            } else {
                                sep_word
                            }
                        );
                        punctuated.push(out);
                        pos += 1;
                    }
                    'I' => {
                        if text.len() == 1 {
                            text[0] = format!("{}{}", text[0], mark_text);
                        } else {
                            let first_word = text.remove(0);
                            text[0] = format!("{}{}{}", first_word, mark_text, text[0]);
                        }
                    }
                    _ => {
                        // Unknown position; treat as I
                        if text.len() == 1 {
                            text[0] = format!("{}{}", text[0], mark_text);
                        } else {
                            let first_word = text.remove(0);
                            text[0] = format!("{}{}{}", first_word, mark_text, text[0]);
                        }
                    }
                }
            }
        }
    }

    punctuated.concat()
}

// ---------------------------------------------------------------------------
// phonemize_to_phones (main entry point)
// ---------------------------------------------------------------------------

/// Phonemize `text` using the espeak backend, preserving and restoring
/// punctuation, then collapse whitespace.
///
/// This is the equivalent of `NeuTTS._to_phones(text)`.
pub fn phonemize_to_phones(espeak: &Espeak, text: &str) -> String {
    // 1. preserve punctuation
    let (chunks, marks) = preserve_punctuation(text);

    // 2. Phonemize each non-empty chunk
    let mut phonemed_chunks: Vec<String> = Vec::new();
    for chunk in &chunks {
        if chunk.is_empty() {
            continue;
        }
        let raw = espeak.text_to_phonemes(chunk);
        // 3. postprocess
        let processed = postprocess_line(&raw);
        if !processed.is_empty() {
            phonemed_chunks.push(processed);
        }
    }

    // 4. restore punctuation
    let restored = restore_punctuation(phonemed_chunks, marks);

    // 5. _to_phones = " ".join(result.split())
    let collapsed: Vec<&str> = restored.split_whitespace().collect();
    collapsed.join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postprocess_line_basic() {
        let input = "h_ə_l_ˈoʊ";
        let result = postprocess_line(input);
        assert_eq!(result, "həlˈoʊ ");
    }

    #[test]
    fn test_postprocess_line_multi_word() {
        let input = "h_ə_l_ˈoʊ w_ˈɜː_l_d";
        let result = postprocess_line(input);
        assert_eq!(result, "həlˈoʊ wˈɜːld ");
    }

    #[test]
    fn test_postprocess_line_double_underscore() {
        let input = "h_ə__l_ˈoʊ";
        let result = postprocess_line(input);
        assert_eq!(result, "həlˈoʊ ");
    }

    #[test]
    fn test_postprocess_line_underscore_space() {
        let input = "h_ə_l_ˈoʊ_ w_ˈɜː_l_d";
        let result = postprocess_line(input);
        assert_eq!(result, "həlˈoʊ wˈɜːld ");
    }

    #[test]
    fn test_postprocess_line_language_switch() {
        let input = "h_ə_l_ˈoʊ (en) w_ˈɜː_l_d";
        let result = postprocess_line(input);
        assert_eq!(result, "həlˈoʊ wˈɜːld ");
    }

    #[test]
    fn test_postprocess_line_empty() {
        assert_eq!(postprocess_line(""), "");
        assert_eq!(postprocess_line("  "), "");
    }

    #[test]
    fn test_preserve_punctuation_hello_world_test() {
        let (chunks, marks) = preserve_punctuation("Hello, world! This is a test.");
        assert_eq!(chunks, vec!["Hello", "world", "This is a test"]);
        assert_eq!(marks.len(), 3);
        assert_eq!(marks[0].mark, ", ");
        assert_eq!(marks[0].position, 'I');
        assert_eq!(marks[1].mark, "! ");
        assert_eq!(marks[1].position, 'I');
        assert_eq!(marks[2].mark, ".");
        assert_eq!(marks[2].position, 'E');
    }

    #[test]
    fn test_preserve_punctuation_ready() {
        let (chunks, marks) = preserve_punctuation("Ready.");
        assert_eq!(chunks, vec!["Ready"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].mark, ".");
        assert_eq!(marks[0].position, 'E');
    }

    #[test]
    fn test_preserve_punctuation_begin() {
        let (chunks, marks) = preserve_punctuation("!Hello world");
        assert_eq!(marks[0].position, 'B');
        assert_eq!(chunks, vec!["Hello world"]);
    }

    #[test]
    fn test_preserve_punctuation_alone() {
        let (chunks, marks) = preserve_punctuation("!!!");
        assert!(chunks.is_empty());
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].position, 'A');
    }

    #[test]
    fn test_preserve_punctuation_no_marks() {
        let (chunks, marks) = preserve_punctuation("Hello world");
        assert_eq!(chunks, vec!["Hello world"]);
        assert!(marks.is_empty());
    }

    #[test]
    fn test_restore_punctuation_hello_world() {
        let chunks = vec!["həlˈoʊ ".to_string(), "wˈɜːld ".to_string(), "ðɪs ɪz ɐ tˈɛst ".to_string()];
        let marks = vec![
            Mark { mark: ", ".to_string(), position: 'I' },
            Mark { mark: "! ".to_string(), position: 'I' },
            Mark { mark: ".".to_string(), position: 'E' },
        ];
        let result = restore_punctuation(chunks, marks);
        assert_eq!(result, "həlˈoʊ, wˈɜːld! ðɪs ɪz ɐ tˈɛst. ");
    }

    #[test]
    fn test_restore_punctuation_ready() {
        let chunks = vec!["ɹˈɛdi ".to_string()];
        let marks = vec![Mark { mark: ".".to_string(), position: 'E' }];
        let result = restore_punctuation(chunks, marks);
        assert_eq!(result, "ɹˈɛdi. ");
    }

    #[test]
    fn test_full_pipeline_hello_world_no_espeak() {
        let text = "Hello, world! This is a test.";
        let (chunks, marks) = preserve_punctuation(text);
        assert_eq!(chunks, vec!["Hello", "world", "This is a test"]);

        // Simulate espeak output for each chunk
        let espeak_outputs = [
            "h_ə_l_ˈoʊ",               // "Hello"
            "w_ˈɜː_l_d",               // "world"
            "ð_ɪ_s ɪ_z ɐ t_ˈɛ_s_t",   // "This is a test"
        ];

        let mut phonemed: Vec<String> = Vec::new();
        for raw in &espeak_outputs {
            let processed = postprocess_line(raw);
            if !processed.is_empty() {
                phonemed.push(processed);
            }
        }
        assert_eq!(phonemed, vec!["həlˈoʊ ", "wˈɜːld ", "ðɪs ɪz ɐ tˈɛst "]);

        let restored = restore_punctuation(phonemed, marks);
        let result: String = restored.split_whitespace().collect::<Vec<&str>>().join(" ");
        assert_eq!(result, "həlˈoʊ, wˈɜːld! ðɪs ɪz ɐ tˈɛst.");
    }

    #[test]
    fn test_full_pipeline_ready_no_espeak() {
        let text = "Ready.";
        let (chunks, marks) = preserve_punctuation(text);
        assert_eq!(chunks, vec!["Ready"]);

        let espeak_outputs = ["ɹ_ˈɛ_d_i"];
        let mut phonemed: Vec<String> = Vec::new();
        for raw in &espeak_outputs {
            let processed = postprocess_line(raw);
            if !processed.is_empty() {
                phonemed.push(processed);
            }
        }
        assert_eq!(phonemed, vec!["ɹˈɛdi "]);

        let restored = restore_punctuation(phonemed, marks);
        let result: String = restored.split_whitespace().collect::<Vec<&str>>().join(" ");
        assert_eq!(result, "ɹˈɛdi.");
    }

    #[test]
    fn test_preserve_dr_smith() {
        let (chunks, marks) = preserve_punctuation("Dr. Smith paid $5.50 on 3/4/2025 — what a deal!");
        assert_eq!(chunks, vec!["Dr", "Smith paid $5", "50 on 3/4/2025", "what a deal"]);
        assert_eq!(marks.len(), 4);
        assert_eq!(marks[0].mark, ". ");
        assert_eq!(marks[0].position, 'I');
        assert_eq!(marks[1].mark, ".");
        assert_eq!(marks[1].position, 'I');
        assert_eq!(marks[2].mark, " — ");
        assert_eq!(marks[2].position, 'I');
        assert_eq!(marks[3].mark, "!");
        assert_eq!(marks[3].position, 'E');
    }

    #[test]
    fn test_preserve_cant_believe() {
        let (chunks, marks) = preserve_punctuation("I can't believe it's already 10am.");
        assert_eq!(chunks, vec!["I can't believe it's already 10am"]);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].mark, ".");
        assert_eq!(marks[0].position, 'E');
    }

    #[test]
    fn test_preserve_parens() {
        let (chunks, marks) = preserve_punctuation("Hello (world)");
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].mark, " (");
        assert_eq!(marks[0].position, 'I');
        assert_eq!(marks[1].mark, ")");
        assert_eq!(marks[1].position, 'E');
        assert_eq!(chunks, vec!["Hello", "world"]);
    }

    #[test]
    fn test_restore_begin_mark() {
        let chunks = vec!["həlˈoʊ ".to_string()];
        let marks = vec![Mark { mark: "!".to_string(), position: 'B' }];
        let result = restore_punctuation(chunks, marks);
        assert_eq!(result, "!həlˈoʊ ");
    }

    #[test]
    fn test_restore_alone_mark() {
        let chunks: Vec<String> = vec![];
        let marks = vec![Mark { mark: "!!!".to_string(), position: 'A' }];
        let result = restore_punctuation(chunks, marks);
        assert_eq!(result, "!!!");
    }

    #[test]
    fn test_postprocess_ready() {
        let input = "ɹ_ˈɛ_d_i";
        let result = postprocess_line(input);
        assert_eq!(result, "ɹˈɛdi ");
    }

    #[test]
    fn test_postprocess_this_is_a_test() {
        let input = "ð_ɪ_s ɪ_z ɐ t_ˈɛ_s_t";
        let result = postprocess_line(input);
        assert_eq!(result, "ðɪs ɪz ɐ tˈɛst ");
    }

    #[test]
    fn test_preserve_em_dash() {
        let (chunks, marks) = preserve_punctuation("Hello — world");
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].mark, " — ");
        assert_eq!(marks[0].position, 'I');
        assert_eq!(chunks, vec!["Hello", "world"]);
    }
}

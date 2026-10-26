// Copyright 2025 Oxide Computer Company

use std::collections::{BTreeMap, BTreeSet, HashSet};

use heck::ToPascalCase;
use log::debug;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use schemars::schema::{
    ArrayValidation, InstanceType, Metadata, ObjectValidation, Schema, SchemaObject, SingleOrVec,
    StringValidation, SubschemaValidation,
};
use unicode_ident::{is_xid_continue, is_xid_start};

use crate::{validate::schema_value_validate, Error, Name, RefKey, Result, TypeSpace};

pub(crate) fn metadata_description(metadata: &Option<Box<Metadata>>) -> Option<String> {
    metadata
        .as_ref()
        .and_then(|metadata| metadata.description.as_ref().cloned())
}

pub(crate) fn metadata_title(metadata: &Option<Box<Metadata>>) -> Option<String> {
    metadata
        .as_ref()
        .and_then(|metadata| metadata.title.as_ref().cloned())
}

pub(crate) fn metadata_title_and_description(metadata: &Option<Box<Metadata>>) -> Option<String> {
    metadata
        .as_ref()
        .and_then(|metadata| match (&metadata.title, &metadata.description) {
            (Some(t), Some(d)) => Some(format!("{}\n\n{}", t, d)),
            (Some(t), None) => Some(t.clone()),
            (None, Some(d)) => Some(d.clone()),
            (None, None) => None,
        })
}

/// Format Markdown descriptions for emission as Rust documentation.
///
/// Call this at the documentation-emission boundary, never on stored metadata,
/// naming inputs, or schema JSON. It adds inline-code styling to code-like
/// identifiers, indents lazy list continuations, and wraps unambiguous bare
/// HTTP(S) URLs in Markdown autolinks without changing their text or destination.
///
/// Source-range edits preserve existing code, links, HTML, escapes, tables and
/// footnote definitions. URL tokens with ambiguous punctuation, non-ASCII text,
/// or unsupported authority syntax are left unchanged. Tab-based and quoted
/// list continuations are not normalized. If edits would change parsed display
/// beyond inline-code styling and visible-URL autolinks, the input is returned
/// unchanged. No Markdown is globally reserialized.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     typify_impl::render_doc_markdown("Use snake_case. See https://example.org/docs"),
///     "Use `snake_case`. See <https://example.org/docs>",
/// );
/// ```
pub fn render_doc_markdown(description: &str) -> String {
    let mut insertions = BTreeMap::<usize, String>::new();
    let mut items = Vec::new();
    let mut code_depth = 0;
    let mut link_depth = 0;
    let mut quote_depth = 0;
    let mut html_elements = Vec::new();
    let mut verbatim_depth = 0;

    for (event, range) in doc_markdown_parser(description).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_) | Tag::FootnoteDefinition(_)) => verbatim_depth += 1,
            Event::End(TagEnd::Table | TagEnd::FootnoteDefinition) => verbatim_depth -= 1,
            // Pipes can occur inside a text event but still delimit cells in
            // rustdoc. Footnote definitions can also be link definitions when
            // extensions are disabled. Preserve both forms' source verbatim.
            _ if verbatim_depth > 0 => (),
            Event::Start(Tag::CodeBlock(_)) => code_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_depth -= 1,
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => link_depth += 1,
            Event::End(TagEnd::Link | TagEnd::Image) => link_depth -= 1,
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Start(Tag::Item) => {
                let line_start = description[..range.start].rfind('\n').map_or(0, |i| i + 1);
                let marker = &description[range.start..range.end];
                let marker_len = if marker.starts_with(['-', '*', '+']) {
                    1
                } else {
                    marker.bytes().take_while(u8::is_ascii_digit).count() + 1
                };
                let spaces = marker[marker_len..]
                    .bytes()
                    .take_while(|b| *b == b' ')
                    .count();
                // Tabs and blockquote prefixes need container-aware column
                // handling; leave these uncommon cases unchanged.
                let prefix = &description[line_start..range.start];
                let indent = (prefix.bytes().all(|b| b == b' ') && (1..=4).contains(&spaces))
                    .then_some(prefix.len() + marker_len + spaces);
                items.push(indent);
            }
            Event::End(TagEnd::Item) => {
                items.pop();
            }
            Event::InlineHtml(html) => {
                // Unlike HTML blocks, inline HTML has no parser container
                // events. Protect text inside raw elements, especially <code>.
                let tag = html.strip_prefix('<').unwrap_or("");
                let closing = tag.starts_with('/');
                let tag = tag.trim_start_matches('/');
                let name = tag
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !name.is_empty() {
                    if closing {
                        if let Some(index) = html_elements.iter().rposition(|n| *n == name) {
                            html_elements.truncate(index);
                        }
                    } else if !html.ends_with("/>")
                        && !matches!(
                            name.as_str(),
                            "area"
                                | "base"
                                | "br"
                                | "col"
                                | "embed"
                                | "hr"
                                | "img"
                                | "input"
                                | "link"
                                | "meta"
                                | "param"
                                | "source"
                                | "track"
                                | "wbr"
                        )
                    {
                        html_elements.push(name);
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak
                if code_depth == 0 && quote_depth == 0 && html_elements.is_empty() =>
            {
                if let Some(Some(indent)) = items.last() {
                    if let Some(newline) = description[range.clone()].rfind('\n') {
                        let start = range.start + newline + 1;
                        let line = description[start..].split('\n').next().unwrap_or("");
                        let spaces = line.bytes().take_while(|b| *b == b' ').count();
                        if spaces < *indent && !line[spaces..].starts_with(['\t', '\r']) {
                            insertions.insert(start, " ".repeat(indent - spaces));
                        }
                    }
                }
            }
            Event::Text(text)
                if code_depth == 0
                    && link_depth == 0
                    && quote_depth == 0
                    && html_elements.is_empty()
                    && description[range.clone()] == *text =>
            {
                for url in text.split_whitespace().filter(|word| doc_bare_url(word)) {
                    let start = range.start + url.as_ptr() as usize - text.as_ptr() as usize;
                    let end = start + url.len();
                    // Require a complete source token within this unformatted
                    // Text event, never a fragment split by escapes, Markdown,
                    // apostrophes or smart punctuation.
                    if description[..start]
                        .chars()
                        .next_back()
                        .is_none_or(char::is_whitespace)
                        && description[end..]
                            .chars()
                            .next()
                            .is_none_or(char::is_whitespace)
                    {
                        insertions.entry(start).or_default().push('<');
                        insertions.entry(end).or_default().push('>');
                    }
                }
                for original in text.split(|c: char| c.is_whitespace() || c == '\'') {
                    let trim = |c: char| !c.is_alphanumeric() && c != ':';
                    let mut word = original.trim_end_matches(trim);
                    if original
                        .get(..word.len() + 2)
                        .is_some_and(|s| s.ends_with("()"))
                    {
                        word = &original[..word.len() + 2];
                    }
                    word = word.trim_start_matches(trim);
                    if !word.starts_with("::") {
                        word = word.trim_start_matches(':');
                    }
                    if !word.ends_with("::") {
                        word = word.trim_end_matches(':');
                    }
                    if doc_word_needs_code(word) {
                        let offset = word.as_ptr() as usize - text.as_ptr() as usize;
                        let source_offset = range.start + offset;
                        // Apostrophes and smart punctuation may split a valid
                        // URL across words or parser events. Inspect its whole
                        // source token before formatting any fragment of it.
                        let before = description[..source_offset]
                            .rsplit(char::is_whitespace)
                            .next()
                            .unwrap_or("");
                        let after = description[source_offset..]
                            .split(char::is_whitespace)
                            .next()
                            .unwrap_or("");
                        let token =
                            &description[source_offset - before.len()..source_offset + after.len()];
                        if doc_word_is_uri(token.trim_start_matches(trim)) {
                            continue;
                        }
                        insertions
                            .entry(range.start + offset)
                            .or_default()
                            .push('`');
                        insertions
                            .entry(range.start + offset + word.len())
                            .or_default()
                            .push('`');
                    }
                }
            }
            _ => (),
        }
    }

    let mut rendered = String::with_capacity(description.len());
    let mut previous = 0;
    for (offset, insertion) in insertions {
        rendered.push_str(&description[previous..offset]);
        rendered.push_str(&insertion);
        previous = offset;
    }
    rendered.push_str(&description[previous..]);
    if rendered != description && doc_display_events(&rendered) != doc_display_events(description) {
        description.to_string()
    } else {
        rendered
    }
}

fn doc_markdown_parser(markdown: &str) -> Parser<'_> {
    use pulldown_cmark::Options;

    // Match Rust 1.95 rustdoc's rendering extensions, including compact tables.
    Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_SMART_PUNCTUATION,
    )
}

fn doc_display_events(markdown: &str) -> Vec<Event<'static>> {
    // Inline-code styling and canonical visible-URL autolinks are intentional.
    // Decoded text, block structure, HTML and other links must be identical.
    let mut events = Vec::new();
    let mut text = String::new();
    let mut visible_url_link = false;
    for (event, range) in doc_markdown_parser(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::Link {
                link_type: pulldown_cmark::LinkType::Autolink,
                ref dest_url,
                ..
            }) if doc_bare_url(dest_url) && markdown[range] == format!("<{dest_url}>") => {
                // Canonical HTTP(S) autolinks display their exact destination.
                // Do not erase ordinary link structure or independently chosen
                // labels/targets: only this visible-URL wrapper is permitted.
                visible_url_link = true;
            }
            Event::End(TagEnd::Link) if visible_url_link => visible_url_link = false,
            Event::Text(part) | Event::Code(part) => text.push_str(&part),
            event => {
                if !text.is_empty() {
                    events.push(Event::Text(std::mem::take(&mut text).into()));
                }
                events.push(event.into_static());
            }
        }
    }
    if !text.is_empty() {
        events.push(Event::Text(text.into()));
    }
    events
}

fn doc_bare_url(word: &str) -> bool {
    let Some(rest) = word
        .strip_prefix("https://")
        .or_else(|| word.strip_prefix("http://"))
    else {
        return false;
    };
    // Avoid guessing where sentence punctuation ends and the URL starts/ends.
    // This conservative ASCII subset covers ordinary documentation URLs while
    // leaving quotes, parentheses, commas, markup and unusual URIs verbatim.
    if !word
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~:/?#@!$&+;=%".contains(&b))
        || !word.ends_with(|c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '~'))
    {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some((host, port)) = authority.split_once(':') {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        host
    } else {
        authority
    };
    host.split('.').all(|label| {
        label.starts_with(|c: char| c.is_ascii_alphanumeric())
            && label.ends_with(|c: char| c.is_ascii_alphanumeric())
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn doc_word_is_uri(word: &str) -> bool {
    // URLs (including non-HTTP schemes and bare email addresses) are not code.
    // Existing Markdown links/autolinks are excluded by the parser above.
    let uri = word.split_once(':').is_some_and(|(scheme, rest)| {
        // A label such as "SharePoint:" has no URI payload. Its colon must
        // not prevent the identifier before it from receiving code styling.
        !rest.is_empty()
            && !rest.starts_with(':')
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    uri || word.contains("://") || word.starts_with("www.") || word.contains('@')
}

fn doc_word_needs_code(word: &str) -> bool {
    if doc_word_is_uri(word) || word.contains('`') {
        return false;
    }
    let underscore = word.contains('_') && word != "_";
    if underscore && word.contains('-') {
        return false;
    }
    let singular = if let Some(prefix) = word.strip_suffix("es").filter(|prefix| {
        prefix.chars().all(|c| c.is_ascii_uppercase()) && prefix.ends_with(['S', 'X'])
    }) {
        prefix
    } else if let Some(prefix) = word
        .strip_suffix("ified")
        .filter(|prefix| prefix.chars().all(|c| c.is_ascii_uppercase()))
    {
        prefix
    } else {
        word.strip_suffix('s').unwrap_or(word)
    };
    let camel_case = !singular.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && singular.chars().all(char::is_alphanumeric)
        && singular
            .chars()
            .filter(|c| c.is_uppercase())
            .take(2)
            .count()
            == 2
        && singular.chars().any(char::is_lowercase);
    underscore || word.contains("::") || word.ends_with("()") || camel_case
}

/// Check if all schemas are mutually exclusive.
///
/// TODO This is used to turn an `anyOf` into a `oneOf` (i.e. if true). However
/// a better approach is probably to use the (newish) merge logic to try to
/// explode an `anyOf` into its 2^N possibilities. Some of those may be invalid
/// in which case we'll end up looking like a `oneOf`. The logic of merging is
/// conceptually identical to the logic below that validates **if** the schemas
/// **could** be merged (i.e. if they're compatible).
pub(crate) fn all_mutually_exclusive(
    subschemas: &[Schema],
    definitions: &BTreeMap<RefKey, Schema>,
) -> bool {
    let len = subschemas.len();
    // With fewer than two subschemas, this is a degenerate case where the
    // lone schema is mutually exclusive with everything else (which happens
    // to be nothing).
    if len < 2 {
        return true;
    }
    // Consider all pairs
    (0..len - 1)
        .flat_map(|ii| (ii + 1..len).map(move |jj| (ii, jj)))
        .all(|(ii, jj)| {
            let a = resolve(&subschemas[ii], definitions);
            let b = resolve(&subschemas[jj], definitions);
            schemas_mutually_exclusive(a, b, definitions)
        })
}

/// This function needs to necessarily be conservative. We'd much prefer a
/// false negative than a false positive.
fn schemas_mutually_exclusive(
    a: &Schema,
    b: &Schema,
    definitions: &BTreeMap<RefKey, Schema>,
) -> bool {
    match (a, b) {
        // If either matches nothing then they are exclusive.
        (Schema::Bool(false), _) => true,
        (_, Schema::Bool(false)) => true,

        // If either matches anything then they are not exclusive.
        (Schema::Bool(true), _) => false,
        (_, Schema::Bool(true)) => false,

        // Iterate over subschemas.
        (
            other,
            Schema::Object(SchemaObject {
                metadata: None,
                instance_type: None,
                format: None,
                enum_values: None,
                const_value: None,
                subschemas: Some(subschemas),
                number: None,
                string: None,
                array: None,
                object: None,
                reference: None,
                extensions: _,
            }),
        )
        | (
            Schema::Object(SchemaObject {
                metadata: None,
                instance_type: None,
                format: None,
                enum_values: None,
                const_value: None,
                subschemas: Some(subschemas),
                number: None,
                string: None,
                array: None,
                object: None,
                reference: None,
                extensions: _,
            }),
            other,
        ) => match subschemas.as_ref() {
            // For an allOf, *any* subschema incompatibility means that the
            // schemas are mutually exclusive.
            SubschemaValidation {
                all_of: Some(s),
                any_of: None,
                one_of: None,
                not: None,
                if_schema: None,
                then_schema: None,
                else_schema: None,
            } => s
                .iter()
                .any(|sub| schemas_mutually_exclusive(sub, other, definitions)),

            // For a oneOf or anyOf, *all* subschemas need to be incompatible.
            SubschemaValidation {
                all_of: None,
                any_of: Some(s),
                one_of: None,
                not: None,
                if_schema: None,
                then_schema: None,
                else_schema: None,
            }
            | SubschemaValidation {
                all_of: None,
                any_of: None,
                one_of: Some(s),
                not: None,
                if_schema: None,
                then_schema: None,
                else_schema: None,
            } => s
                .iter()
                .all(|sub| schemas_mutually_exclusive(sub, other, definitions)),

            // For a not, they're mutually exclusive if they *do* match.
            SubschemaValidation {
                all_of: None,
                any_of: None,
                one_of: None,
                not: Some(sub),
                if_schema: None,
                then_schema: None,
                else_schema: None,
            } => !schemas_mutually_exclusive(sub, other, definitions),

            // Assume other subschemas are complex to understand and may
            // therefore be compatible.
            _ => false,
        },

        // If one schema has enumerated values, it's incompatible if all values
        // fail to validate against the other schema. (Conversely, if all
        // values *do* validate against the other schema, it simply seems
        // redundant--the most interesting case is if *some* values validate
        // and others do not.)
        (
            schema,
            Schema::Object(SchemaObject {
                instance_type: None,
                enum_values: Some(enum_values),
                ..
            }),
        )
        | (
            Schema::Object(SchemaObject {
                instance_type: None,
                enum_values: Some(enum_values),
                ..
            }),
            schema,
        ) => enum_values
            .iter()
            .all(|value| schema_value_validate(schema, value, definitions).is_err()),

        // If one schema has a constant value, it's incompatible if that value
        // fails to validate against the other schema.
        (
            schema,
            Schema::Object(SchemaObject {
                const_value: Some(value),
                ..
            }),
        )
        | (
            Schema::Object(SchemaObject {
                const_value: Some(value),
                ..
            }),
            schema,
        ) => schema_value_validate(schema, value, definitions).is_err(),

        // Neither is a Schema::Bool; we need to look at the instance types.
        (Schema::Object(a), Schema::Object(b)) => {
            match (&a.instance_type, &b.instance_type) {
                // If either is None, assume we're dealing with a more complex
                // type and that they are not exclusive.
                (None, _) => false,
                (_, None) => false,

                // If each schema has a single type and they aren't the same
                // then the types must be mutually exclusive.
                (Some(SingleOrVec::Single(a_single)), Some(SingleOrVec::Single(b_single)))
                    if a_single != b_single =>
                {
                    true
                }

                // For two objects we need to check required properties and
                // additional properties to see if there exists an object that
                // could successfully be validated by either schema.
                (Some(SingleOrVec::Single(a_single)), Some(SingleOrVec::Single(b_single)))
                    if a_single == b_single && a_single.as_ref() == &InstanceType::Object =>
                {
                    if let (
                        SchemaObject {
                            metadata: _,
                            instance_type: _,
                            format: None,
                            enum_values: None,
                            const_value: None,
                            subschemas: None,
                            number: None,
                            string: None,
                            array: None,
                            object: Some(a_validation),
                            reference: None,
                            extensions: _,
                        },
                        SchemaObject {
                            metadata: _,
                            instance_type: _,
                            format: None,
                            enum_values: None,
                            const_value: None,
                            subschemas: None,
                            number: None,
                            string: None,
                            array: None,
                            object: Some(b_validation),
                            reference: None,
                            extensions: _,
                        },
                    ) = (a, b)
                    {
                        object_schemas_mutually_exclusive(a_validation, b_validation)
                    } else {
                        // Could check further, but we'll be conservative.
                        false
                    }
                }

                // For two objects we need to check required properties and
                // additional properties to see if there exists an object that
                // could successfully be validated by either schema.
                (Some(SingleOrVec::Single(a_single)), Some(SingleOrVec::Single(b_single)))
                    if a_single == b_single && a_single.as_ref() == &InstanceType::Array =>
                {
                    if let (
                        SchemaObject {
                            metadata: _,
                            instance_type: _,
                            format: None,
                            enum_values: None,
                            const_value: None,
                            subschemas: None,
                            number: None,
                            string: None,
                            array: Some(a_validation),
                            object: None,
                            reference: None,
                            extensions: _,
                        },
                        SchemaObject {
                            metadata: _,
                            instance_type: _,
                            format: None,
                            enum_values: None,
                            const_value: None,
                            subschemas: None,
                            number: None,
                            string: None,
                            array: Some(b_validation),
                            object: None,
                            reference: None,
                            extensions: _,
                        },
                    ) = (a, b)
                    {
                        array_schemas_mutually_exclusive(a_validation, b_validation, definitions)
                    } else {
                        // Could check further, but we'll be conservative.
                        false
                    }
                }

                // For other simple types, check if the single type is the same
                // or not.
                (Some(SingleOrVec::Single(a_single)), Some(SingleOrVec::Single(b_single))) => {
                    a_single != b_single
                }

                // For two schemas with lists of instance types, make sure that
                // all pairs differ.
                (Some(SingleOrVec::Vec(a_vec)), Some(SingleOrVec::Vec(b_vec))) => a_vec
                    .iter()
                    .all(|instance_type| !b_vec.contains(instance_type)),

                // If one is a single type and the other is a vec, it will
                // suffice for now to check that the single item is different
                // than all the items in the vec.
                (Some(SingleOrVec::Single(single)), Some(SingleOrVec::Vec(vec)))
                | (Some(SingleOrVec::Vec(vec)), Some(SingleOrVec::Single(single))) => {
                    !vec.contains(single)
                }
            }
        }
    }
}

// See if there are unique, required properties of each that cannot be present
// in the other. In other words, see if there are properties that would
// uniquely identify an objects as validating exclusively with one or the other
// (but not with both).
fn object_schemas_mutually_exclusive(
    a_validation: &ObjectValidation,
    b_validation: &ObjectValidation,
) -> bool {
    let ObjectValidation {
        required: a_required,
        properties: a_properties,
        ..
    } = a_validation;
    let ObjectValidation {
        required: b_required,
        properties: b_properties,
        ..
    } = b_validation;

    // No properties? Too permissive / insufficiently exclusive.
    if a_properties.is_empty() || b_properties.is_empty() {
        return false;
    }

    // Either set of required properties must not be a subset of the other's
    // properties i.e. if there's a property that *must* be in one of the two
    // objects, and *cannot* be in the other, a property whose presence or
    // absence determines which of the two objects is relevant.
    if !a_required.is_subset(&b_properties.keys().cloned().collect())
        || !b_required.is_subset(&a_properties.keys().cloned().collect())
    {
        true
    } else {
        // Even if all required properties of each is a permitted property of
        // the other, each may have required properties that have fixed values
        // that differ. This can happen in particular for internally or
        // adjacently tagged enums where the properties may be identical but
        // the value of the tag property will be unique.

        // Compute the set that consists of fixed-value properties--a
        // tuple of the property name and the fixed value. Note that we may
        // encounter objects that specify that a field is required, but do
        // *not* specify the field. That's ok, but we can't assure mutual
        // exclusivity.
        let aa = a_required
            .iter()
            .filter_map(|name| {
                let t = a_properties.get(name).unwrap();
                constant_string_value(t).map(|s| (name.clone(), s))
            })
            .collect::<HashSet<_>>();
        let bb = b_required
            .iter()
            .filter_map(|name| {
                let t = b_properties.get(name).unwrap();
                constant_string_value(t).map(|s| (name.clone(), s))
            })
            .collect::<HashSet<_>>();

        // True if neither is a subset of the other.
        !aa.is_subset(&bb) && !bb.is_subset(&aa)
    }
}

fn array_schemas_mutually_exclusive(
    a_validation: &ArrayValidation,
    b_validation: &ArrayValidation,
    definitions: &BTreeMap<RefKey, Schema>,
) -> bool {
    match (a_validation, b_validation) {
        // If one is an array with a single item type and the other is a tuple
        // of a fixed size with fixed item types, we could only see a conflict
        // if the single item was compatible with *all* types of the tuple.
        // It's therefore sufficient to see if it's exclusive with *any* of the
        // types of the tuple.
        (
            ArrayValidation {
                items: Some(SingleOrVec::Single(single)),
                additional_items: None,
                ..
            },
            ArrayValidation {
                items: Some(SingleOrVec::Vec(vec)),
                additional_items: None,
                max_items: Some(max_items),
                min_items: Some(min_items),
                unique_items: None,
                contains: None,
            },
        )
        | (
            ArrayValidation {
                items: Some(SingleOrVec::Vec(vec)),
                additional_items: None,
                max_items: Some(max_items),
                min_items: Some(min_items),
                unique_items: None,
                contains: None,
            },
            ArrayValidation {
                items: Some(SingleOrVec::Single(single)),
                additional_items: None,
                ..
            },
        ) if max_items == min_items && *max_items as usize == vec.len() => vec
            .iter()
            .any(|schema| schemas_mutually_exclusive(schema, single, definitions)),

        (aa, bb) => {
            // If min > max then these schemas are incompatible.
            match (&aa.max_items, &bb.min_items) {
                (Some(max), Some(min)) if min > max => return true,
                _ => (),
            }
            match (&bb.max_items, &aa.min_items) {
                (Some(max), Some(min)) if min > max => return true,
                _ => (),
            }

            match (&aa.items, &aa.max_items, &bb.items, &bb.max_items) {
                // If thee's a single item schema and it's mutually exclusive
                // then we're done.
                (Some(SingleOrVec::Single(a_items)), _, Some(SingleOrVec::Single(b_items)), _)
                    if schemas_mutually_exclusive(a_items, b_items, definitions) =>
                {
                    return true;
                }

                _ => (),
            }
            debug!(
                "giving up on mutual exclusivity check {} {}",
                serde_json::to_string_pretty(aa).unwrap(),
                serde_json::to_string_pretty(bb).unwrap(),
            );
            false
        }
    }
}

/// If this schema represents a constant-value string, return that string,
/// otherwise return None.
pub(crate) fn constant_string_value(schema: &Schema) -> Option<&str> {
    match schema {
        // Singleton, typed enumerated value.
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: Some(SingleOrVec::Single(single)),
            format: None,
            enum_values: Some(values),
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) if single.as_ref() == &InstanceType::String && values.len() == 1 => {
            values.first().unwrap().as_str()
        }

        // Singleton, untyped enumerated value.
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: None,
            format: None,
            enum_values: Some(values),
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) if values.len() == 1 => values.first().unwrap().as_str(),

        // Constant value.
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: Some(SingleOrVec::Single(single)),
            format: None,
            enum_values: None,
            const_value: Some(value),
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) if single.as_ref() == &InstanceType::String => value.as_str(),

        // Constant, untyped value.
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: Some(value),
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) => value.as_str(),

        _ => None,
    }
}

fn decode_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

pub(crate) fn ref_key(ref_name: &str) -> RefKey {
    if ref_name == "#" {
        RefKey::Root
    } else if let Some(idx) = ref_name.rfind('/') {
        let decoded_segment = decode_segment(&ref_name[idx + 1..]);

        RefKey::Def(decoded_segment)
    } else {
        panic!("expected a '/' in $ref: {}", ref_name)
    }
}

fn resolve<'a>(
    schema: &'a Schema,
    definitions: &'a std::collections::BTreeMap<RefKey, Schema>,
) -> &'a Schema {
    match schema {
        Schema::Bool(_) => schema,
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: Some(ref_name),
            extensions: _,
        }) => definitions.get(&ref_key(ref_name)).unwrap(),
        Schema::Object(SchemaObject {
            reference: None, ..
        }) => schema,
        // TODO Not sure what this would mean...
        _ => todo!(),
    }
}

/// Determine if a schema has a name (potentially).
pub(crate) fn schema_is_named(schema: &Schema) -> Option<String> {
    let raw_name = match schema {
        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: None,
            reference: Some(reference),
            extensions: _,
        }) => {
            let idx = reference.rfind('/')?;
            Some(reference[idx + 1..].to_string())
        }

        Schema::Object(SchemaObject {
            metadata: Some(metadata),
            ..
        }) if metadata.as_ref().title.is_some() => Some(metadata.as_ref().title.as_ref()?.clone()),

        Schema::Object(SchemaObject {
            metadata: _,
            instance_type: _,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: Some(subschemas),
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) => singleton_subschema(subschemas).and_then(schema_is_named),

        // Best-effort fallback for things with raw types that can be easily inferred
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(single)),
            format,
            ..
        }) => match (**single, format.as_deref()) {
            (_, Some(format)) => Some(format.to_pascal_case()),
            (InstanceType::Boolean, _) => Some("Boolean".to_string()),
            (InstanceType::Integer, _) => Some("Integer".to_string()),
            (InstanceType::Number, _) => Some("Number".to_string()),
            (InstanceType::String, _) => Some("String".to_string()),
            (InstanceType::Array, _) => Some("Array".to_string()),
            (InstanceType::Object, _) => Some("Object".to_string()),
            (InstanceType::Null, _) => Some("Null".to_string()),
        },

        _ => None,
    }?;

    Some(sanitize(&raw_name, Case::Pascal))
}

/// Return the object data or None if it's not an object (or doesn't conform to
/// the objects we know how to handle).
pub(crate) fn get_object(schema: &Schema) -> Option<(&Option<Box<Metadata>>, &ObjectValidation)> {
    match schema {
        // Object
        Schema::Object(SchemaObject {
            metadata,
            instance_type: Some(SingleOrVec::Single(single)),
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: None,
            number: _,
            string: _,
            array: _,
            object: Some(validation),
            reference: None,
            extensions: _,
        }) if single.as_ref() == &InstanceType::Object
            && schema_none_or_false(&validation.additional_properties)
            && validation.max_properties.is_none()
            && validation.min_properties.is_none()
            && validation.pattern_properties.is_empty()
            && validation.property_names.is_none() =>
        {
            Some((metadata, validation.as_ref()))
        }
        // Object with no explicit type (but the proper validation)
        Schema::Object(SchemaObject {
            metadata,
            instance_type: None,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: None,
            number: None,
            string: None,
            array: None,
            object: Some(validation),
            reference: None,
            extensions: _,
        }) if schema_none_or_false(&validation.additional_properties)
            && validation.max_properties.is_none()
            && validation.min_properties.is_none()
            && validation.pattern_properties.is_empty()
            && validation.property_names.is_none() =>
        {
            Some((metadata, validation.as_ref()))
        }

        // Trivial (n == 1) subschemas
        Schema::Object(SchemaObject {
            metadata,
            instance_type: _,
            format: None,
            enum_values: None,
            const_value: None,
            subschemas: Some(subschemas),
            number: None,
            string: None,
            array: None,
            object: None,
            reference: None,
            extensions: _,
        }) => singleton_subschema(subschemas).and_then(|sub_schema| {
            get_object(sub_schema).map(|(m, validation)| match m {
                Some(_) => (metadata, validation),
                None => (&None, validation),
            })
        }),

        // None if the schema doesn't match the shape we expect.
        _ => None,
    }
}

// We infer from a Some(Schema::Bool(false)) or None value that either nothing
// or nothing of importance is in the additional properties.
fn schema_none_or_false(additional_properties: &Option<Box<Schema>>) -> bool {
    matches!(
        additional_properties.as_ref().map(Box::as_ref),
        None | Some(Schema::Bool(false))
    )
}

pub(crate) fn singleton_subschema(subschemas: &SubschemaValidation) -> Option<&Schema> {
    match subschemas {
        SubschemaValidation {
            all_of: Some(subschemas),
            any_of: None,
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        }
        | SubschemaValidation {
            all_of: None,
            any_of: Some(subschemas),
            one_of: None,
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        }
        | SubschemaValidation {
            all_of: None,
            any_of: None,
            one_of: Some(subschemas),
            not: None,
            if_schema: None,
            then_schema: None,
            else_schema: None,
        } if subschemas.len() == 1 => subschemas.first(),
        _ => None,
    }
}

pub(crate) enum Case {
    Pascal,
    Snake,
}

pub(crate) fn sanitize(input: &str, case: Case) -> String {
    use heck::{ToPascalCase, ToSnakeCase};
    let to_case = match case {
        Case::Pascal => str::to_pascal_case,
        Case::Snake => str::to_snake_case,
    };

    // If every case was special then none of them would be.
    let out = match input {
        "+1" => "plus1".to_string(),
        "-1" => "minus1".to_string(),
        _ => to_case(&input.replace("'", "").replace(|c| !is_xid_continue(c), "-")),
    };

    let prefix = to_case("x");

    let out = match out.chars().next() {
        None => prefix,
        Some(c) if is_xid_start(c) => out,
        Some(_) => format!("{}{}", prefix, out),
    };

    // Make sure the string is a valid Rust identifier.
    if accept_as_ident(&out) {
        out
    } else {
        format!("{}_", out)
    }
}

/// Return true if the string is a valid Rust identifier.
///
/// If this function returns false, typify adds a trailing underscore to it. For
/// example, `fn` becomes `fn_`.
pub fn accept_as_ident(ident: &str) -> bool {
    // Adapted from https://docs.rs/syn/2.0.114/src/syn/ident.rs.html#60-74. The
    // main change is adding `gen` to the list.
    match ident {
        "_" |
        // Based on https://doc.rust-lang.org/1.65.0/reference/keywords.html
        "abstract" | "as" | "async" | "await" | "become" | "box" | "break" |
        "const" | "continue" | "crate" | "do" | "dyn" | "else" | "enum" |
        "extern" | "false" | "final" | "fn" | "for" | "gen" | "if" | "impl" |
        "in" | "let" | "loop" | "macro" | "match" | "mod" | "move" | "mut" |
        "override" | "priv" | "pub" | "ref" | "return" | "Self" | "self" |
        "static" | "struct" | "super" | "trait" | "true" | "try" | "type" |
        "typeof" | "unsafe" | "unsized" | "use" | "virtual" | "where" |
        "while" | "yield" => false,
        _ => true,
    }
}

pub(crate) fn recase(input: &str, case: Case) -> (String, Option<String>) {
    let new = sanitize(input, case);
    let rename = if new == input {
        None
    } else {
        Some(input.to_string())
    };
    (new, rename)
}

pub(crate) fn unique<I, T>(items: I) -> bool
where
    I: IntoIterator<Item = T>,
    T: Eq + std::hash::Hash,
{
    let mut unique = HashSet::new();
    items.into_iter().all(|item| unique.insert(item))
}

pub(crate) fn get_type_name(type_name: &Name, metadata: &Option<Box<Metadata>>) -> Option<String> {
    let name = match (type_name, metadata_title(metadata)) {
        (Name::Required(name), _) => name.clone(),
        (Name::Suggested(name), None) => name.clone(),
        (_, Some(name)) => name,
        (Name::Unknown, None) => None?,
    };

    Some(sanitize(&name, Case::Pascal))
}

pub(crate) struct TypePatch {
    pub name: String,
    pub derives: BTreeSet<String>,
    pub attrs: BTreeSet<String>,
}

impl TypePatch {
    /// Creates a new TypePatch by resolving patches for the given type name.
    pub fn new(type_space: &TypeSpace, type_name: String) -> Self {
        match type_space.settings.patch.get(&type_name) {
            None => Self {
                name: type_name,
                derives: Default::default(),
                attrs: Default::default(),
            },

            Some(patch) => {
                let name = patch.rename.clone().unwrap_or(type_name);
                let derives = patch.derives.iter().cloned().collect();
                let attrs = patch.attrs.iter().cloned().collect();

                Self {
                    name,
                    derives,
                    attrs,
                }
            }
        }
    }
}

pub(crate) struct StringValidator {
    max_length: Option<u32>,
    min_length: Option<u32>,
    pattern: Option<regress::Regex>,
}

impl StringValidator {
    pub fn new(type_name: &Name, validation: Option<&StringValidation>) -> Result<Self> {
        let (max_length, min_length, pattern) =
            validation.map_or(Ok((None, None, None)), |validation| {
                let max = validation.max_length;
                let min = validation.min_length;
                let pattern = validation
                    .pattern
                    .as_ref()
                    .map(|pattern| {
                        regress::Regex::new(pattern).map_err(|e| Error::InvalidSchema {
                            type_name: type_name.clone().into_option(),
                            reason: format!("invalid pattern '{}' {}", pattern, e),
                        })
                    })
                    .transpose()?;
                Ok((max, min, pattern))
            })?;
        Ok(Self {
            max_length,
            min_length,
            pattern,
        })
    }

    pub fn is_valid<S: AsRef<str>>(&self, s: S) -> bool {
        // Per the JSON Schema spec (and RFC 8259), minLength/maxLength count
        // Unicode code points, not UTF-8 bytes, so we must not use `len()`.
        self.max_length
            .as_ref()
            .is_none_or(|max| s.as_ref().chars().count() as u32 <= *max)
            && self
                .min_length
                .as_ref()
                .is_none_or(|min| s.as_ref().chars().count() as u32 >= *min)
            && self
                .pattern
                .as_ref()
                .is_none_or(|pattern| pattern.find(s.as_ref()).is_some())
    }
}

/// A re-ordering of JSON schema instance types that puts integer values before
/// number values.
///
/// This is used for untagged enum generation to ensure that integer values
/// are matched before number values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ReorderedInstanceType {
    /// The JSON schema instance type `null`.
    Null,

    /// The JSON schema instance type `boolean`.
    Boolean,

    /// The JSON schema instance type `integer`.
    Integer,

    /// The JSON schema instance type `number`.
    Number,

    /// The JSON schema instance type `string`.
    String,

    /// The JSON schema instance type `array`.
    Array,

    /// The JSON schema instance type `object`.
    Object,
}

impl From<InstanceType> for ReorderedInstanceType {
    fn from(instance_type: InstanceType) -> Self {
        match instance_type {
            InstanceType::Null => Self::Null,
            InstanceType::Boolean => Self::Boolean,
            InstanceType::Object => Self::Object,
            InstanceType::Array => Self::Array,
            InstanceType::Integer => Self::Integer,
            InstanceType::Number => Self::Number,
            InstanceType::String => Self::String,
        }
    }
}

impl From<ReorderedInstanceType> for InstanceType {
    fn from(instance_type: ReorderedInstanceType) -> Self {
        match instance_type {
            ReorderedInstanceType::Null => Self::Null,
            ReorderedInstanceType::Boolean => Self::Boolean,
            ReorderedInstanceType::Object => Self::Object,
            ReorderedInstanceType::Array => Self::Array,
            ReorderedInstanceType::Integer => Self::Integer,
            ReorderedInstanceType::Number => Self::Number,
            ReorderedInstanceType::String => Self::String,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use schemars::{
        gen::{SchemaGenerator, SchemaSettings},
        schema::StringValidation,
        schema_for, JsonSchema,
    };

    use crate::{
        util::{
            all_mutually_exclusive, decode_segment, sanitize, schemas_mutually_exclusive, Case,
            ReorderedInstanceType,
        },
        Name,
    };

    use super::StringValidator;

    fn assert_doc_rendering(source: &str, expected: &str) {
        let actual = super::render_doc_markdown(source);
        assert_eq!(actual, expected);
        assert_eq!(
            super::render_doc_markdown(&actual),
            actual,
            "not idempotent"
        );
        let render = |parser: pulldown_cmark::Parser<'_>| {
            let mut autolinks = Vec::new();
            let events = parser.inspect(|event| {
                if let pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
                    link_type: pulldown_cmark::LinkType::Autolink,
                    dest_url,
                    ..
                }) = event
                {
                    if super::doc_bare_url(dest_url) {
                        autolinks.push(dest_url.to_string());
                    }
                }
            });
            let mut html = String::new();
            pulldown_cmark::html::push_html(&mut html, events);
            for url in autolinks {
                let visible = url.replace('&', "&amp;");
                html = html.replace(&format!("<a href=\"{visible}\">{visible}</a>"), &visible);
            }
            // New inline-code styling is intentional; all rendered text,
            // structure, HTML, link destinations and fenced code must agree.
            html.replace("<code>", "").replace("</code>", "")
        };
        assert_eq!(
            render(super::doc_markdown_parser(source)),
            render(super::doc_markdown_parser(&actual)),
            "rustdoc rendered meaning changed"
        );
        assert_eq!(
            render(pulldown_cmark::Parser::new(source)),
            render(pulldown_cmark::Parser::new(&actual)),
            "default Markdown rendered meaning changed"
        );
    }

    #[test]
    fn test_doc_markdown_identifiers() {
        assert_doc_rendering(
            "OpenAI's ChatGPT uses SharePoint, snake_case, 'response.mcp_call.failed', std::fmt::Display and parse_json(). IDs, APIs, CPUs, JSON and UTF-8 stay prose.",
            "`OpenAI`'s `ChatGPT` uses `SharePoint`, `snake_case`, '`response.mcp_call.failed`', `std::fmt::Display` and `parse_json()`. IDs, APIs, CPUs, JSON and UTF-8 stay prose.",
        );
        assert_doc_rendering(
            "Includes details on how the model used the web (search, open_page, find_in_page).",
            "Includes details on how the model used the web (search, `open_page`, `find_in_page`).",
        );
    }

    #[test]
    fn test_doc_markdown_authentic_connector_description() {
        // Unmodified openai-8243cc8.json:
        // components/schemas/MCPTool/properties/connector_id/description
        // (also identical in BetaMCPTool).
        let source = "Identifier for service connectors, like those available in ChatGPT. One of\n\
`server_url`, `connector_id`, or `tunnel_id` must be provided. Learn more\n\
about service connectors [here](https://developers.openai.com/api/docs/guides/tools-connectors-mcp#connectors).\n\
\n\
This field is deprecated for models released after September 1, 2026.\n\
Use `server_url` to connect to a remote MCP server, or `tunnel_id` to\n\
connect through a Secure MCP Tunnel.\n\
\n\
Currently supported `connector_id` values are:\n\
\n\
- Dropbox: `connector_dropbox`\n\
- Gmail: `connector_gmail`\n\
- Google Calendar: `connector_googlecalendar`\n\
- Google Drive: `connector_googledrive`\n\
- Microsoft Teams: `connector_microsoftteams`\n\
- Outlook Calendar: `connector_outlookcalendar`\n\
- Outlook Email: `connector_outlookemail`\n\
- SharePoint: `connector_sharepoint`\n";
        let expected = source
            .replace("ChatGPT", "`ChatGPT`")
            .replace("SharePoint:", "`SharePoint`:");
        assert_eq!(crate::render_doc_markdown(source), expected);
        assert_doc_rendering(source, &expected);
    }

    #[test]
    fn test_doc_markdown_preserves_rustdoc_tables() {
        for table in [
            "a_b|c_d\n---|---\nx_y|z_a",
            "| FooBar | snake_case |\n| --- | --- |\n| `existing_code` | x_y |\n",
            "- Item\n\n  a_b|c_d\n  ---|---\n  x_y|z_a\n",
        ] {
            assert!(super::doc_markdown_parser(table).any(|event| matches!(
                event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Table(_))
            )));
            assert_doc_rendering(table, table);
        }
        assert_doc_rendering(
            "FooBar before.\n\na_b|c_d\n---|---\nx_y|z_a\n\nOpenAI after.",
            "`FooBar` before.\n\na_b|c_d\n---|---\nx_y|z_a\n\n`OpenAI` after.",
        );
        assert_doc_rendering(
            "FooBar[^some_note]\n\n[^some_note]: snake_case\n    FooBar",
            "`FooBar`[^some_note]\n\n[^some_note]: snake_case\n    FooBar",
        );
    }

    #[test]
    fn test_doc_markdown_preserves_complete_bare_urls() {
        for url in [
            "https://example.org/foo'bar_baz",
            "https://example.org/foo'FooBar",
            "https://example.org/foo?key='bar_baz'&other=FooBar",
            "(https://example.org/foo'bar_baz).",
            "urn:example:foo'bar_baz",
            "www.example.org/foo'bar_baz",
            "mailto:foo'bar_baz@example.org",
        ] {
            assert_doc_rendering(url, url);
            assert_doc_rendering(
                &format!("OpenAI uses {url} and snake_case."),
                &format!("`OpenAI` uses {url} and `snake_case`."),
            );
        }
    }

    #[test]
    fn test_doc_markdown_links_unambiguous_bare_http_urls() {
        assert_eq!(
            super::doc_display_events("https://example.org/docs"),
            super::doc_display_events("<https://example.org/docs>"),
        );
        assert_ne!(
            super::doc_display_events("[label](https://example.org/original)"),
            super::doc_display_events("[label](https://example.org/changed)"),
            "the semantic guard must retain independently chosen link targets",
        );
        assert_doc_rendering(
            "The OpenAI REST API. Please see https://platform.openai.com/docs/api-reference for more details.\n\nhttps://openai.com/policies/terms-of-use",
            "The `OpenAI` REST API. Please see <https://platform.openai.com/docs/api-reference> for more details.\n\n<https://openai.com/policies/terms-of-use>",
        );
        assert_doc_rendering(
            "- See https://example.org/foo_bar?key=value&other=next\nlazy continuation",
            "- See <https://example.org/foo_bar?key=value&other=next>\n  lazy continuation",
        );
        let protected = concat!(
            "`https://example.org/docs` [label](https://example.org/docs) ",
            "<https://example.org/docs> ![image](https://example.org/docs)\n\n",
            "<span>https://example.org/docs</span>\n\n",
            "```text\nhttps://example.org/docs\n```\n\n",
            "url|value\n---|---\nhttps://example.org/docs|x\n\n",
            "Footnote[^note]\n\n[^note]: https://example.org/docs\n",
        );
        assert_doc_rendering(protected, protected);
        for ambiguous in [
            "https://example.org/docs.",
            "(https://example.org/docs)",
            "https://example.org/foo'bar_baz",
            "https://example.org/café",
            "https://example.org/foo\\_bar",
            "https://example.org/a&amp;b",
            "https://[::1]/docs",
            "https://user@example.org/docs",
            "https://example.org/a,b",
            "https://example.org/path?",
        ] {
            assert_doc_rendering(ambiguous, ambiguous);
        }
    }

    #[test]
    fn test_doc_markdown_preserves_existing_markdown() {
        let unchanged = concat!(
            "`snake_case` and ``code_with ` embedded`` and `multi_line\ncode_span`.\n\n",
            "[OpenAI snake_case](https://example.com/foo_bar \"FooBar\") and ",
            "![FooBar](image_file.png) and <https://example.com/foo_bar>.\n\n",
            "[FooBar reference][some_ref]\n\n",
            "[some_ref]: https://example.com/foo_bar \"FooBar\"\n\n",
            "Bare https://example.com/foo_bar?q=FooBar, www.example.com/foo_bar, ",
            "mailto:some_one@example.com and urn:some_id remain unchanged.\n\n",
            "Escaped snake\\_case, \\`literal\\`, &amp; &#95;, café, 日本語 and 🦀.\n\n",
            "<code>snake_case OpenAI</code> <span title=\"FooBar\">snake_case</span><br>\n\n",
            "<div>\nFooBar snake_case\n</div>\n\n",
            "```json\n{\"description\": \"OpenAI snake_case\"}\n```\n\n",
            "~~~text\nOpenAI snake_case\n~~~\n\n",
            "    OpenAI snake_case\n",
        );
        assert_doc_rendering(unchanged, unchanged);
        assert_doc_rendering(
            "**FooBar** and *snake_case*, café 🦀 and `already_code`.",
            "**`FooBar`** and *`snake_case`*, café 🦀 and `already_code`.",
        );
    }

    #[test]
    fn test_doc_markdown_lazy_list_continuations() {
        assert_doc_rendering(
            "- `stream`: If set to true, the model response data will be streamed\nas it is generated using [server-sent events](https://example.com/foo_bar).\nSee the [Streaming section below](https://example.com/streaming-events)\nfor more information.\n- `limit`: A limit between\n1 and 100, and the default is 20.\n\nOutside the list.",
            "- `stream`: If set to true, the model response data will be streamed\n  as it is generated using [server-sent events](https://example.com/foo_bar).\n  See the [Streaming section below](https://example.com/streaming-events)\n  for more information.\n- `limit`: A limit between\n  1 and 100, and the default is 20.\n\nOutside the list.",
        );
        assert_doc_rendering(
            "1. Outer item\nlazy line\n   - Inner item\nlazy inner\n   - Next item\n     already indented\n\nAfterwards.",
            "1. Outer item\n   lazy line\n   - Inner item\n     lazy inner\n   - Next item\n     already indented\n\nAfterwards.",
        );
        assert_doc_rendering(
            "- Hard break  \ncontinuation\n- Backslash break\\\ncontinuation\n",
            "- Hard break  \n  continuation\n- Backslash break\\\n  continuation\n",
        );
    }

    #[test]
    fn test_doc_markdown_list_code_and_html_are_untouched() {
        assert_doc_rendering(
            "- FooBar wraps\nto snake_case.\n\n  ```json\n  {\"description\": \"OpenAI snake_case\"}\n  ```\n\n  ~~~text\n  snake_case\n  ~~~\n\n      indented_code\n\n  <div>\n  snake_case\n  </div>\n\n- Last item\n",
            "- `FooBar` wraps\n  to `snake_case`.\n\n  ```json\n  {\"description\": \"OpenAI snake_case\"}\n  ```\n\n  ~~~text\n  snake_case\n  ~~~\n\n      indented_code\n\n  <div>\n  snake_case\n  </div>\n\n- Last item\n",
        );
        let unchanged = "- `snake_case\ncode_span`\n\n> - FooBar\n>   snake_case\n\n- <code>snake_case\nOpenAI</code>\n";
        assert_doc_rendering(unchanged, unchanged);
    }

    #[test]
    fn test_doc_markdown_ambiguous_backticks_preserve_display() {
        for unchanged in [
            "snake_case`existing_code`",
            "`existing_code`FooBar",
            "An unmatched ` before OpenAI",
        ] {
            assert_doc_rendering(unchanged, unchanged);
        }
        assert_doc_rendering(
            "OpenAI followed by an unmatched `",
            "`OpenAI` followed by an unmatched `",
        );
        assert_doc_rendering(
            "- FooBar\r\nlazy continuation\r\n",
            "- `FooBar`\r\n  lazy continuation\r\n",
        );
    }

    #[test]
    fn test_doc_markdown_does_not_change_metadata_or_names() {
        let metadata = Some(Box::new(schemars::schema::Metadata {
            title: Some("FooBar".to_string()),
            description: Some("OpenAI snake_case".to_string()),
            ..Default::default()
        }));
        let original = metadata.clone();
        assert_eq!(
            super::metadata_description(&metadata).as_deref(),
            Some("OpenAI snake_case")
        );
        assert_eq!(
            super::metadata_title_and_description(&metadata).as_deref(),
            Some("FooBar\n\nOpenAI snake_case")
        );
        assert_eq!(
            super::get_type_name(&Name::Unknown, &metadata).as_deref(),
            Some("FooBar")
        );
        let _ = super::render_doc_markdown(&super::metadata_description(&metadata).unwrap());
        assert_eq!(metadata, original);
    }

    #[test]
    fn test_doc_markdown_enum_emission_keeps_raw_descriptions() {
        use crate::type_entry::{
            StructProperty, StructPropertyRename, StructPropertyState, Variant, VariantDetails,
        };
        let mut types = crate::TypeSpace::default();
        let string_type = types.add_type(&schema_for!(String).schema.into()).unwrap();
        let variant = Variant {
            raw_name: "Example".to_string(),
            ident_name: Some("Example".to_string()),
            description: Some("OpenAI snake_case".to_string()),
            details: VariantDetails::Struct(vec![StructProperty {
                name: "value".to_string(),
                rename: StructPropertyRename::None,
                state: StructPropertyState::Required,
                description: Some("- FooBar\nlazy continuation".to_string()),
                type_id: string_type,
            }]),
        };
        let original = variant.clone();
        let output = crate::enums::output_variant(
            &variant,
            &types,
            &mut crate::output::OutputSpace::default(),
            "ExampleEnum",
        )
        .to_string();
        assert!(output.contains(&quote::quote! { #[doc = "`OpenAI` `snake_case`"] }.to_string()));
        assert!(output
            .contains(&quote::quote! { #[doc = "- `FooBar`\n  lazy continuation"] }.to_string()));
        assert_eq!(variant, original);
    }

    #[test]
    fn test_non_exclusive_structs() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        struct A {
            a: Option<()>,
            b: (),
        }

        #[derive(JsonSchema)]
        struct B {
            a: (),
            b: Option<()>,
        }

        let a = schema_for!(A).schema.into();
        let b = schema_for!(B).schema.into();

        assert!(!schemas_mutually_exclusive(&a, &b, &BTreeMap::new()));
        assert!(!schemas_mutually_exclusive(&b, &a, &BTreeMap::new()));
    }

    #[test]
    fn test_non_exclusive_oneof_subschema() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        enum A {
            B(i32),
            C(i64),
        }

        let mut settings = SchemaSettings::default();
        settings.inline_subschemas = true;
        let gen = SchemaGenerator::new(settings);

        let a = gen.into_root_schema_for::<Vec<A>>().schema.into();

        assert!(!schemas_mutually_exclusive(&a, &a, &BTreeMap::new()));
    }

    #[test]
    fn test_unique_prop_structs() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        struct A {
            a: Option<()>,
            b: (),
        }

        #[derive(JsonSchema)]
        struct B {
            a: (),
            b: Option<()>,
            c: (),
        }

        let a = schema_for!(A).schema.into();
        let b = schema_for!(B).schema.into();

        assert!(schemas_mutually_exclusive(&a, &b, &BTreeMap::new()));
        assert!(schemas_mutually_exclusive(&b, &a, &BTreeMap::new()));
    }

    #[test]
    fn test_exclusive_structs() {
        #![allow(dead_code)]

        #[derive(JsonSchema)]
        struct A {
            a: Option<()>,
            b: (),
            aa: (),
        }

        #[derive(JsonSchema)]
        struct B {
            a: (),
            b: Option<()>,
            bb: (),
        }

        let a = schema_for!(A).schema.into();
        let b = schema_for!(B).schema.into();

        assert!(schemas_mutually_exclusive(&a, &b, &BTreeMap::new()));
        assert!(schemas_mutually_exclusive(&b, &a, &BTreeMap::new()));
    }

    #[test]
    fn test_exclusive_simple_arrays() {
        let a = schema_for!(Vec<u32>).schema.into();
        let b = schema_for!(Vec<f32>).schema.into();

        assert!(schemas_mutually_exclusive(&a, &b, &BTreeMap::new()));
        assert!(schemas_mutually_exclusive(&b, &a, &BTreeMap::new()));
    }

    #[test]
    fn test_all_mutually_exclusive_empty() {
        assert!(all_mutually_exclusive(&[], &BTreeMap::new()));
    }

    #[test]
    fn test_decode_segment() {
        assert_eq!(decode_segment("foo~1bar"), "foo/bar");
        assert_eq!(decode_segment("foo~0bar"), "foo~bar");
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("type", Case::Snake), "type_");
        assert_eq!(sanitize("ref", Case::Snake), "ref_");
        assert_eq!(sanitize("gen", Case::Snake), "gen_");
        assert_eq!(sanitize("gen", Case::Pascal), "Gen");
        assert_eq!(sanitize("+1", Case::Snake), "plus1");
        assert_eq!(sanitize("-1", Case::Snake), "minus1");
        assert_eq!(sanitize("@timestamp", Case::Pascal), "Timestamp");
        assert_eq!(sanitize("won't and can't", Case::Pascal), "WontAndCant");
        assert_eq!(
            sanitize(
                "urn:ietf:params:scim:schemas:extension:gluu:2.0:user_",
                Case::Pascal
            ),
            "UrnIetfParamsScimSchemasExtensionGluu20User"
        );
        assert_eq!(sanitize("Ipv6Net", Case::Snake), "ipv6_net");
        assert_eq!(sanitize("V6", Case::Pascal), "V6");
    }

    #[test]
    fn test_string_validation() {
        let permissive = StringValidator::new(&Name::Unknown, None).unwrap();
        assert!(permissive.is_valid("everything should be fine"));
        assert!(permissive.is_valid(""));

        let also_permissive = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: None,
                min_length: None,
                pattern: None,
            }),
        )
        .unwrap();
        assert!(also_permissive.is_valid("everything should be fine"));
        assert!(also_permissive.is_valid(""));

        let eight = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: Some(8),
                min_length: Some(8),
                pattern: None,
            }),
        )
        .unwrap();
        assert!(eight.is_valid("Shadrach"));
        assert!(!eight.is_valid("Meshach"));
        assert!(eight.is_valid("Abednego"));

        let ach = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: None,
                min_length: None,
                pattern: Some("ach$".to_string()),
            }),
        )
        .unwrap();
        assert!(ach.is_valid("Shadrach"));
        assert!(ach.is_valid("Meshach"));
        assert!(!ach.is_valid("Abednego"));
    }

    #[test]
    fn test_string_validation_multi_byte() {
        // minLength/maxLength count Unicode code points, not UTF-8 bytes.
        // "héllo" is 5 code points but 6 bytes (the "é" is 2 bytes).
        let five = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: Some(5),
                min_length: Some(5),
                pattern: None,
            }),
        )
        .unwrap();
        assert!(five.is_valid("héllo"));
        assert!(five.is_valid("hello"));
        // 6 bytes but only 5 code points -- should still be valid.
        assert_eq!("héllo".len(), 6);
        assert_eq!("héllo".chars().count(), 5);

        // A single 4-byte emoji is one code point, so it satisfies a
        // minLength/maxLength of 1 even though it is 4 bytes long.
        let one = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: Some(1),
                min_length: Some(1),
                pattern: None,
            }),
        )
        .unwrap();
        assert!(one.is_valid("🍔"));
        assert_eq!("🍔".len(), 4);
        assert_eq!("🍔".chars().count(), 1);
        // Two code points should now be rejected by maxLength: 1.
        assert!(!one.is_valid("🍔🍔"));

        // A string of exactly 8 code points, each multi-byte, should pass
        // an exact-length-8 validator even though its byte length is 24.
        let eight = StringValidator::new(
            &Name::Unknown,
            Some(&StringValidation {
                max_length: Some(8),
                min_length: Some(8),
                pattern: None,
            }),
        )
        .unwrap();
        let emoji8 = "🍔".repeat(8);
        assert_eq!(emoji8.len(), 32);
        assert_eq!(emoji8.chars().count(), 8);
        assert!(eight.is_valid(&emoji8));
        assert!(!eight.is_valid("🍔".repeat(7)));
        assert!(!eight.is_valid("🍔".repeat(9)));
    }

    #[test]
    fn test_instance_type_ordering() {
        let null = ReorderedInstanceType::Null;
        let boolean = ReorderedInstanceType::Boolean;
        let integer = ReorderedInstanceType::Integer;
        let number = ReorderedInstanceType::Number;
        let string = ReorderedInstanceType::String;
        let array = ReorderedInstanceType::Array;
        let object = ReorderedInstanceType::Object;

        assert!(null < boolean);
        assert!(boolean < integer);
        assert!(integer < number);
        assert!(number < string);
        assert!(string < array);
        assert!(array < object);
    }
}

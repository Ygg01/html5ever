// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::btree_map::BTreeMap;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::io::ErrorKind::InvalidData;
use std::io::{self, Error, Write};

use log::warn;
pub use markup5ever::serialize::{AttrRef, Serialize, Serializer, TraversalScope};
use markup5ever::{is_void_element, local_name, namespace_prefix, namespace_url, ns};
use markup5ever::{EqStr, LocalName, Namespace, Prefix};

use crate::util::{is_name_char, is_name_start_char, is_xml_char};
use crate::QualName;

type LocalPrefixMap = HashMap<Prefix, Namespace>;

#[derive(Clone, Debug)]
/// [Namespace Prefix Map](https://w3c.github.io/DOM-Parsing/#the-namespace-prefix-map)
///
/// As defined in W3C Editor's Draft. `NamespacePrefixMap`, associates
/// unique key value that is either `Namespace` or `None` with an ordered list of associated prefix values.
/// The `NamespacePrefixMap` will be populated by previously seen Namespaces#
/// and all their previously encountered prefix associations for a given node and its ancestors.
///
/// Important preconditions: If the `NamespacePrefixMap` contains a namespace (or a None value),
/// the attached candidates list will not be empty. Also all missing value for prefix are replaced
/// with prefix for empty string (e.g.`Prefix::from("")`).
pub struct NamespacePrefixMap {
    map: BTreeMap<Option<Namespace>, Vec<Prefix>>,
}

impl Default for NamespacePrefixMap {
    fn default() -> Self {
        NamespacePrefixMap {
            map: {
                let mut namespace_map = BTreeMap::default();
                namespace_map.insert(Some(ns!(xml)), vec![namespace_prefix!("xml")]);
                namespace_map
            },
        }
    }
}

impl NamespacePrefixMap {
    /// Constructs a new empty `NamespacePrefixMap`
    pub fn new() -> NamespacePrefixMap {
        NamespacePrefixMap {
            map: BTreeMap::default(),
        }
    }

    /// [Retrieves preferred prefix](https://w3c.github.io/DOM-Parsing/#dfn-retrieving-a-preferred-prefix-string)
    ///
    /// If for given namespace a candidates list exist, returns either `preferred_prefix` if found
    /// or the last `Prefix` in candidates list.
    ///
    /// If for given namespace no candidates list exist, returns `None`.
    pub fn retrieve_preferred_prefix(
        &self,
        ns: &Option<Namespace>,
        preferred_prefix: &Prefix,
    ) -> Option<Prefix> {
        if let Some(candidates_list) = self.map.get(ns) {
            // Since the candidates list will always contain at least one element, this
            // will never panic.
            let mut last_prefix = &candidates_list[0];
            for prefix in candidates_list {
                last_prefix = prefix;
                if prefix == preferred_prefix {
                    break;
                }
            }
            return Some(last_prefix.clone());
        }
        None
    }

    /// [Finds a prefix](https://w3c.github.io/DOM-Parsing/#dfn-found).
    ///
    /// If it finds any element for given namespace that matches given prefix, it will return
    /// `true` for first prefix that matches given prefix.
    ///
    /// Otherwise returns `false`.
    pub fn find_prefix(&self, ns: &Option<Namespace>, prefix: &Prefix) -> bool {
        if let Some(candidates_list) = self.map.get(ns) {
            return candidates_list.iter().any(|pref| pref == prefix);
        }
        false
    }
    /// [Adds a prefix to `NamespacePrefixMap`](https://w3c.github.io/DOM-Parsing/#dfn-add)
    ///
    /// If there already is an ordered list associated with that `Namespace` key,
    /// it adds the Prefix to the end of the list.
    ///
    /// Otherwise it creates an empty `list`, adds `Prefix` to the end of that `list`,
    /// and inserts an entry with given `Namespace` as key, and `list` as value.
    pub fn add(&mut self, ns: Option<Namespace>, prefix: Prefix) {
        if let Some(candidates_list) = self.map.get_mut(&ns) {
            candidates_list.push(prefix);
        } else {
            let candidate_list = vec![prefix];
            self.map.insert(ns, candidate_list);
        }
    }
}

/// Helper function for mapping an Option<T> to T
///
/// Usually used to map Option<Atom> to T
fn map_opt_atom<'a, T: From<&'a str> + Clone>(opt: &Option<T>) -> T {
    if let Some(atom) = opt {
        atom.clone()
    } else {
        T::from("")
    }
}

/// Helper function for determine if an Option contains a given value
fn opt_eq<T: PartialEq>(opt: &Option<T>, cmp: &T) -> bool {
    match opt {
        Some(val) => val == cmp,
        _ => false,
    }
}

/// Check that all chars in string match function.
/// Exits early on first wrong character spotted
fn matches_rules(haystack: &str, needle_fn: fn(char) -> bool) -> bool {
    for c in haystack.chars() {
        if !needle_fn(c) {
            return false;
        }
    }
    true
}

fn convert_fmt_to_io_error(_err: std::fmt::Error) -> std::io::Error {
    Error::new(InvalidData, format!("Error writing to string"))
}

/// Function for [generating a namespace prefixes](https://w3c.github.io/DOM-Parsing/#dfn-generating-a-prefix)
///
/// Generates a prefix given a namespace prefix map - map, a string new_namespace and a reference
/// to a Serializer opts, containing generated namespace prefix index, returning a generated prefix
fn generate_prefix(
    map: &mut NamespacePrefixMap,
    new_namespace: &Namespace,
    opts: &mut SerializeOpts,
) -> Prefix {
    let generated_prefix = Prefix::from(format!("ns{}", opts.prefix_index));
    opts.prefix_index += 1;
    map.add(Some(new_namespace.clone()), generated_prefix.clone());
    generated_prefix
}

#[derive(Clone)]
/// Struct for setting serializer options.
pub struct SerializeOpts {
    /// Serialize the root node? Default: ChildrenOnly
    pub traversal_scope: TraversalScope,
    /// Flag require well-formed? Default: true
    pub require_well_formed: bool,
    /// Default context namespace of serialized document. Default: None
    pub context_namespace: Option<Namespace>,
    /// Prefix map passed to serializer. Defaults: Empty map
    pub prefix_map: NamespacePrefixMap,
    /// Generated namespace prefix index. Defaults: 1u32
    pub prefix_index: u32,
}

impl Default for SerializeOpts {
    fn default() -> SerializeOpts {
        SerializeOpts {
            traversal_scope: TraversalScope::ChildrenOnly(None),
            require_well_formed: true,
            context_namespace: None,
            prefix_map: NamespacePrefixMap::default(),
            prefix_index: 1,
        }
    }
}

/// Method for serializing generic node to a given writer.
pub fn serialize<Wr, T>(writer: Wr, node: &T, opts: SerializeOpts) -> io::Result<()>
where
    Wr: Write,
    T: Serialize,
{
    let traversal_scope = opts.traversal_scope.clone();
    let mut ser = XmlSerializer::new(writer, opts);
    node.serialize(&mut ser, traversal_scope)
}

struct ElemInfo {
    skip_end_tag: bool,
    qualified_name: String,
}

impl Default for ElemInfo {
    fn default() -> Self {
        ElemInfo {
            skip_end_tag: false,
            qualified_name: String::new(),
        }
    }
}

/// Struct used for serializing nodes into a text that other XML
/// parses can read.
///
/// Serializer contains a set of functions (`start_elem`, `end_elem`...)
/// that make parsing nodes easier.
pub struct XmlSerializer<Wr> {
    writer: Wr,
    opts: SerializeOpts,
    skip_end_tag: bool,
    stack: Vec<ElemInfo>,
}

impl<Wr: Write> XmlSerializer<Wr> {
    /// Creates a new Serializier from a writer and given serialization options.
    pub fn new(writer: Wr, opts: SerializeOpts) -> Self {
        XmlSerializer {
            writer,
            opts,
            skip_end_tag: false,
            stack: Vec::new(),
        }
    }

    /// Writes given text into the Serializer, escaping it,
    /// depending on where the text is written inside the tag or attribute value.
    ///
    /// For example
    ///```text
    ///    <tag>'&-quotes'</tag>   becomes      <tag>'&amp;-quotes'</tag>
    ///    <tag = "'&-quotes'">    becomes      <tag = "&apos;&amp;-quotes&apos;"
    ///```
    fn write_escaped_text(&mut self, text: &str) -> io::Result<()> {
        for c in text.chars() {
            match c {
                '&' => self.writer.write_all(b"&amp;"),
                '>' => self.writer.write_all(b"&gt;"),
                '<' => self.writer.write_all(b"&lt;"),
                c => {
                    if self.opts.require_well_formed {
                        if !is_xml_char(c) {
                            return Err(Error::new(
                                InvalidData,
                                format!("Character `{}` is not a valid XML", c),
                            ));
                        }
                    }
                    self.writer.write_fmt(format_args!("{}", c))
                }
            }?;
        }
        Ok(())
    }

    /// [Serializing an attribute value](https://w3c.github.io/DOM-Parsing/#dfn-serializing-an-attribute-value)
    ///
    /// Following algorithm serializes the attributes given an attribute value and require well-formed flag
    /// which is part of XmlSerializer definition (accessed through self reference).
    fn serialize_attr_value(&mut self, attr_value: &str) -> io::Result<()> {
        for c in attr_value.chars() {
            match c {
                '&' => self.writer.write_all(b"&amp;"),
                '>' => self.writer.write_all(b"&gt;"),
                '<' => self.writer.write_all(b"&lt;"),
                '"' => self.writer.write_all(b"&quot;"),
                c => {
                    if self.opts.require_well_formed {
                        if !is_xml_char(c) {
                            return Err(Error::new(
                                InvalidData,
                                format!("Character `{}` is not a valid XML", c),
                            ));
                        }
                    }
                    self.writer.write_fmt(format_args!("{}", c))
                }
            }?;
        }
        Ok(())
    }

    /// [Recording namespace information](https://w3c.github.io/DOM-Parsing/#recording-the-namespace)
    ///
    /// The following function updates the `NamespacePrefixMap` with any found
    /// namespace prefix definitions, adds the found prefix definition to the
    /// local prefixes map and returns a local default namespace if there
    /// is one.
    fn recording_namespace_information(
        &self,
        map: &mut NamespacePrefixMap,
        local_prefixes_map: &mut LocalPrefixMap,
        attrs: &Vec<AttrRef>,
    ) -> Option<Namespace> {
        let mut default_namespace_attr_value = None;

        for attr in attrs {
            let attribute_namespace = &attr.0.ns;
            let attribute_prefix = &attr.0.prefix;

            if attribute_namespace.eq_str("xmlns") {
                match attribute_prefix {
                    None => default_namespace_attr_value = Some(Namespace::from(attr.1)),
                    Some(_prefix) => {
                        let prefix_definition = Prefix::from(&*attr.0.local);
                        let namespace_definition = attr.1;

                        if namespace_definition == &ns!(xml) {
                            continue;
                        }

                        let namespace_definition = if namespace_definition == "" {
                            None
                        } else {
                            Some(Namespace::from(namespace_definition))
                        };

                        if map.find_prefix(&namespace_definition, &prefix_definition) {
                            continue;
                        }

                        map.add(namespace_definition.clone(), prefix_definition.clone());
                        local_prefixes_map
                            .insert(prefix_definition, map_opt_atom(&namespace_definition));
                    }
                }
            }
        }
        default_namespace_attr_value
    }

    /// [Serializing an Element's attributes](https://w3c.github.io/DOM-Parsing/#serializing-an-element-s-attributes)
    ///
    /// The following function performs XML serialization of the Element's attributes
    /// for a given `elem` represented by name `QualName` and a list of XML `attributes`
    /// as well as a `NamespacePrefixMap` map, `LocalPrefixMap`, and `ignore_namespace_definition` flag.
    fn serialize_elem_attrs(
        &mut self,
        attributes: Vec<AttrRef>,
        map: &mut NamespacePrefixMap,
        local_prefixes_map: LocalPrefixMap,
        ignore_namespace_definition: bool,
    ) -> io::Result<()> {
        let mut localname_set: HashSet<(&Namespace, &LocalName)> = HashSet::new();

        for attr in attributes {
            let tuple = (&attr.0.ns, &attr.0.local);
            if self.opts.require_well_formed && localname_set.contains(&tuple) {
                return Err(Error::new(
                    InvalidData,
                    format!("Serialization of attribute {:?} would fail to produce a well-formed element ", tuple),
                ));
            }
            // Create a new tuple and add it to localname set.
            localname_set.insert(tuple);
            let attribute_namespace = &attr.0.ns;
            let mut candidate_prefix = None;

            // If attribute space is not null (i.e. empty)
            if attribute_namespace != &ns!() {
                let search_prefix = match &attr.0.prefix {
                    Some(t) => t.clone(),
                    _ => Prefix::from(""),
                };
                candidate_prefix =
                    map.retrieve_preferred_prefix(&Some(attr.0.ns.clone()), &search_prefix);

                if attribute_namespace == &ns!(xmlns) {
                    let redeclare_xml_namespace = Namespace::from(attr.1) == ns!(xml);
                    let ignored_ns_def = ignore_namespace_definition && attr.0.prefix.is_none();
                    let redefine_namespace = match &attr.0.prefix {
                        Some(prefix) => {
                            let local_name = Prefix::from(&*attr.0.local);
                            let ns_attr_value = Some(Namespace::from(&*attr.1));
                            let found_in_local = local_prefixes_map.get(&local_name);
                            let previously_defined = self
                                .opts
                                .prefix_map
                                .find_prefix(&ns_attr_value, &local_name);

                            let attr_local_not_in_local_pref_map = found_in_local.is_none();
                            let attr_local_in_already = found_in_local.is_some()
                                && found_in_local.cloned().ne(&ns_attr_value);

                            (attr_local_in_already || attr_local_not_in_local_pref_map)
                                && previously_defined
                        }
                        _ => false,
                    };

                    if redeclare_xml_namespace || ignored_ns_def || redefine_namespace {
                        continue;
                    }

                    if self.opts.require_well_formed {
                        let attr_value = Namespace::from(&*attr.1);
                        if attr_value == ns!(xmlns) {
                            return Err(Error::new(
                                InvalidData,
                                format!("Creation of XMLNS namespace(`{:?}`) is allowed only under strict qualifications", attr.1),
                            ));
                        }
                        if attr_value == ns!() {
                            return Err(Error::new(
                                InvalidData,
                                "Namespace declarations can't be used to undeclare a namespace (use default namespace instead)",
                            ));
                        }
                    }

                    candidate_prefix = Some(namespace_prefix!("xmlns"));
                } else {
                    // Otherwise the attribute namespace is not the XMLNS namespace
                    let generated_prefix =
                        Some(generate_prefix(map, attribute_namespace, &mut self.opts));
                    self.writer.write_all(b" xmlns:")?;
                    self.writer
                        .write_all(generated_prefix.unwrap().as_bytes())?;
                    self.writer.write_all(b"=\"")?;
                    self.serialize_attr_value(&attr.1)?;
                    self.writer.write_all(b"\"")?;
                }
            }

            self.writer.write_all(b" ")?;

            if let Some(candidate_prefix) = candidate_prefix {
                self.writer.write_all(candidate_prefix.as_bytes())?;
                self.writer.write_all(b":")?;
            }

            if self.opts.require_well_formed {
                if attr.0.local.contains(":")
                    || !matches_rules(attr.0.local.as_ref(), is_xml_char)
                    || (attr.0.local.eq("xmlns") && attr.0.ns.eq(""))
                {
                    return Err(Error::new(
                        InvalidData,
                        format!("Serialization of attribute {:?} would fail to produce a well-formed element ", tuple),
                    ));
                }
            }

            self.writer.write_all(attr.0.local.as_bytes())?;
            self.writer.write_all(b"=\"")?;
            self.serialize_attr_value(&attr.1)?;
            self.writer.write_all(b"\"")?;
        }
        Ok(())
    }
}

impl<Wr: Write> Serializer for XmlSerializer<Wr> {
    fn start_elem<'a, AttrIter>(
        &mut self,
        name: QualName,
        attrs: AttrIter,
        leaf_node: bool,
    ) -> io::Result<()>
    where
        AttrIter: Iterator<Item = AttrRef<'a>>,
    {
        let attributes: Vec<AttrRef> = attrs.collect();

        // 3.2.1.1 point 1
        // Check if required well-formed flag is set.
        // If it is, check if local name contains `:` (U+003A COLON) or
        // if local name doesn't with XML name rules.
        if self.opts.require_well_formed {
            if name.local.contains(":") {
                return Err(Error::new(InvalidData, "Local name can't contain `:`."));
            }
            let mut first = true;
            for local_char in name.local.chars() {
                if first {
                    first = false;
                    if !is_name_start_char(local_char) {
                        return Err(Error::new(
                            InvalidData,
                            format!("Local name can't start with `{}`.", local_char),
                        ));
                    }
                } else {
                    if !is_name_char(local_char) {
                        return Err(Error::new(
                            InvalidData,
                            format!("Local name can't contain `{}`.", local_char),
                        ));
                    }
                }
            }
        }
        // 3.2.1.1 point 2
        // Let markup be string "<"
        self.writer.write_all(b"<")?;

        // 3.2.1.1 point 3
        let mut qualified_name = String::new();

        // 3.2.1.1 point 4
        self.skip_end_tag = false;

        // 3.2.1.1 point 5
        let mut ignore_namespace_definition = false;

        // 3.2.1.1 point 6
        // Given prefix map, copy a namespace prefix map into map
        let mut map = self.opts.prefix_map.clone();

        // 3.2.1.1 point 7
        // Local prefix map is an empty map, with unique Node prefix strings as keys
        // and corresponding namespaceURI Node values as the map's key values
        let mut local_prefixes_map = LocalPrefixMap::new();

        // 3.2.1.1. point 8
        let local_default_namespace =
            self.recording_namespace_information(&mut map, &mut local_prefixes_map, &attributes);

        // 3.2.1.1. point 9
        let mut inherited_ns = self.opts.context_namespace.clone();

        // 3.2.1.1. point 10
        // let ns be the value of the node's namespaceURI attribute
        let ns = name.ns.clone();

        match inherited_ns {
            // 3.2.1.1. point 11
            // If inherited_ns is equal to ns
            Some(inherited_ns) if inherited_ns == ns => {
                // if local default namespace is not null, then ignore namespace definition attribute
                if local_default_namespace.is_some() {
                    ignore_namespace_definition = true;
                }
                // If ns is in the XML namespace
                // then append to qualified name "xml"
                // else append node's local name, node prefix is dropped.
                if ns == ns!(xml) {
                    write!(qualified_name, "xml:{}", &name.local)
                        .map_err(convert_fmt_to_io_error)?;
                } else {
                    write!(qualified_name, "{}", &name.local).map_err(convert_fmt_to_io_error)?;
                }
                self.writer.write_all(qualified_name.as_bytes())?;
            }
            // 3.2.1.1. point 12
            // Otherwise, inherited_ns is not equal to ns (the node's
            _ => {
                let mut prefix = name.prefix.clone();
                let mut candidate_prefix =
                    map.retrieve_preferred_prefix(&inherited_ns, &map_opt_atom(&prefix));

                if prefix.eq_str("xmlns") {
                    if self.opts.require_well_formed {
                        return Err(Error::new(InvalidData, format!("An Element with prefix 'xmlns' will not legally round-trip in a conforming XML parser. ")));
                    }
                    candidate_prefix = prefix.clone();
                }
                // Found a suitable namespace prefix
                if let Some(candidate_prefix) = candidate_prefix {
                    // Append the candidate prefix, ':' and node's local name.
                    write!(qualified_name, "{}:{}", candidate_prefix, name.local)
                        .map_err(convert_fmt_to_io_error)?;
                    self.writer.write_all(qualified_name.as_bytes())?;

                    if let Some(ref local_default_namespace) = local_default_namespace {
                        if local_default_namespace == &ns!() {
                            inherited_ns = None;
                        } else if local_default_namespace != &ns!(xml) {
                            inherited_ns = Some(local_default_namespace.clone());
                        }
                    }
                }
                // Otherwise if prefix is not null then
                if let Some(mut some_prefix) = prefix {
                    // If the local prefixes map contains a key matching prefix (non null)
                    // then let prefix be a newly generated prefix
                    if local_prefixes_map.contains_key(&some_prefix) {
                        some_prefix = generate_prefix(&mut map, &ns, &mut self.opts)
                    }

                    map.add(Some(ns.clone()), some_prefix.clone());

                    write!(qualified_name, "{}:{}", some_prefix, name.local)
                        .map_err(convert_fmt_to_io_error)?;
                    self.writer.write_all(qualified_name.as_bytes())?;

                    self.writer.write_all(b" xmlns:")?;
                    self.writer.write_all(some_prefix.as_bytes())?;
                    self.writer.write_all(b"=\"")?;
                    self.serialize_attr_value(&ns)?;
                    self.writer.write_all(b"\"")?;

                    if local_default_namespace.is_some() {
                        inherited_ns = local_default_namespace;
                    }
                }
                // Otherwise, if local default namespace is null, or local default namespace is not null
                // and its value is not equal to ns
                else if local_default_namespace.is_none()
                    || (local_default_namespace.is_some() && opt_eq(&local_default_namespace, &ns))
                {
                    ignore_namespace_definition = true;

                    write!(qualified_name, "{}", name.local).map_err(convert_fmt_to_io_error)?;

                    inherited_ns = Some(ns.clone());

                    self.writer.write_all(qualified_name.as_bytes())?;
                    self.writer.write_all(b" xmlns=\"")?;
                    self.serialize_attr_value(&ns)?;
                    self.writer.write_all(b"\"")?;
                }
                // Node has local default namespace that matches ns
                else if opt_eq(&local_default_namespace, &ns) {
                    write!(qualified_name, "{}", name.local).map_err(convert_fmt_to_io_error)?;
                    inherited_ns = Some(ns.clone());
                    self.writer.write_all(qualified_name.as_bytes())?;
                }
            }
        }
        // 3.2.1.1. point 13
        self.serialize_elem_attrs(
            attributes,
            &mut map,
            local_prefixes_map,
            ignore_namespace_definition,
        )?;

        let ignore_children = ns == ns!(html) && leaf_node && is_void_element(&name.local);
        let empty_node = ns != ns!(html) && leaf_node;

        // 3.2.1.1. point 14
        if ignore_children {
            self.writer.write_all(b" /")?;
            self.skip_end_tag = true;
        }
        // 3.2.1.1. point 15
        else if empty_node {
            self.writer.write_all(b"/")?;
        }

        // 3.2.1.1. point 16
        self.writer.write_all(b">")?;

        self.stack.push(ElemInfo {
            skip_end_tag,
            qualified_name,
        });

        Ok(())
    }

    fn end_elem(&mut self, name: QualName) -> io::Result<()> {
        let info = match self.stack.pop() {
            Some(info) => info,
            None => {
                warn!("missing ElemInfo, creating default.");
                Default::default()
            }
        };

        // 3.2.1.1. point 17
        if info.skip_end_tag {
            return Ok(());
        }

        // 3.2.1.1. point 18
        if name.ns == ns!(html) && name.local == local_name!("template") {
            // TODO DocumentFragment serialization
        }

        // 3.2.1.1. point 20
        self.writer.write_all(b"</")?;
        self.writer.write_all(info.qualified_name.as_bytes())?;
        self.writer.write_all(b">")?;

        Ok(())
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        self.write_escaped_text(text)
    }

    fn write_comment(&mut self, text: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if text.contains("--") {
                return Err(Error::new(
                    InvalidData,
                    "Comment contains double minus `--`",
                ));
            } else if text.ends_with("-") {
                return Err(Error::new(InvalidData, "Comment ends with minus"));
            }
        }
        self.writer.write_all(b"<!--")?;
        self.write_escaped_text(text)?;
        self.writer.write_all(b"-->")
    }

    // TODO Until DTD is fully implemented
    fn write_doctype(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(b"<!DOCTYPE ")?;
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"!>")
    }

    fn write_processing_instruction(&mut self, target: &str, data: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if "xml".eq_ignore_ascii_case(target) {
                return Err(Error::new(
                    InvalidData,
                    "Processing instruction target can't be equal to `xml`.",
                ));
            }
            if target.contains(':') {
                return Err(Error::new(
                    InvalidData,
                    "Processing instruction target contains illegal characters `:`.",
                ));
            }
            if data.contains("?>") {
                return Err(Error::new(
                    InvalidData,
                    "Processing instruction data contains illegal characters `?>`.",
                ));
            }
        }
        self.writer.write_all(b"<?")?;
        self.write_escaped_text(target)?;
        self.writer.write_all(b" ")?;
        self.write_escaped_text(data)?;
        self.writer.write_all(b"?>")
    }
}

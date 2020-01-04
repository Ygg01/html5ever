// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::btree_map::BTreeMap;
use std::collections::HashMap;
use std::io::{self, Write, Error};
use std::io::ErrorKind::InvalidData;

use markup5ever::{Namespace, Prefix};
use markup5ever::{namespace_prefix, namespace_url, ns};
pub use markup5ever::serialize::{AttrRef, Serialize, Serializer, TraversalScope};

use crate::{QualName};
use crate::util::{is_xml_char, is_name_start_char, is_name_char};

type LocalPrefixMap = HashMap<Prefix, Namespace>;

#[derive(Clone, Debug)]
/// [Namespace Prefix Map](https://w3c.github.io/DOM-Parsing/#the-namespace-prefix-map)
/// 
/// As defined in W3C Editor's Draft. `NamespacePrefixMap`, associates
/// unqiue key value that is either `Namespace` or `None` with an ordered list of associated prefix values.
/// The `NamespacePrefixMap` will be populated by previosuly seen Namespaces#
/// and all their previously encountered prefix associations for a given node and its ancestors.
/// 
/// Important preconditions: If the `NamespacePrefixMap` contains a namespace (or a None value), 
/// the attached candidates list will not be empty. Also all missing value for prefix are replaced
/// with prefix for empty string (e.g.`Prefix::from("")`).
pub struct NamespacePrefixMap {
    map: BTreeMap<Option<Namespace>, Vec<Prefix>>,
}

impl NamespacePrefixMap {
    /// Constructs a new empty `NamespacePrefixMap`
    pub fn new() -> NamespacePrefixMap {
        NamespacePrefixMap {
            map: BTreeMap::default(),
        }
    }

    /// Retrieves preferred prefix as per [specification](https://w3c.github.io/DOM-Parsing/#dfn-retrieving-a-preferred-prefix-string)
    /// 
    /// If for given namespace a candidates list exist, returns either `preferred_prefix` if found
    /// or the last `Prefix` in candidates list.alloc
    /// 
    /// If for given namespace no candidates list exist, returns `None`.
    pub fn retrieve_preferred_prefix(&self, ns: &Option<Namespace>, preferred_prefix: &Prefix) -> Option<&Prefix> {
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
            return Some(last_prefix)
        }
        None
    }

    /// Finds a prefix as per [specification](https://w3c.github.io/DOM-Parsing/#dfn-found).
    ///
    /// If it finds any element for given namespace that matches given prefix, it will return
    /// `true` for first prefix that matches given prefix.
    ///
    /// Otherwise returns `false`.
    pub fn find_prefix(&self, ns: &Option<Namespace>, prefix: &Prefix) -> bool {
        if let Some(candidates_list) = self.map.get(ns) {
            return candidates_list.iter().any(|pref| pref == prefix)
        }
        false
    }
    /// Adds a prefix to namespace per [specification](https://w3c.github.io/DOM-Parsing/#dfn-add)
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
            prefix_map: {
                let mut namesepace_map = NamespacePrefixMap::new();
                namesepace_map.add(Some(ns!(xml)), namespace_prefix!("xml"));
                namesepace_map
            },
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

/// Struct used for serializing nodes into a text that other XML
/// parses can read.
///
/// Serializer contains a set of functions (start_elem, end_elem...)
/// that make parsing nodes easier.
pub struct XmlSerializer<Wr> {
    writer: Wr,
    opts: SerializeOpts,
}

impl<Wr: Write> XmlSerializer<Wr> {
    /// Creates a new Serializier from a writer and given serialization options.
    pub fn new(writer: Wr, opts: SerializeOpts) -> Self {

        XmlSerializer {
            writer: writer,
            opts: opts,
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
                        if !is_xml_char(c){
                            return Err(Error::new(InvalidData, format!("Invalid characters = {}", c)));
                        }
                    }
                    self.writer.write_fmt(format_args!("{}", c))
                },
            }?;
        }
        Ok(())
    }


    /// Recording namespace information as [per specification](https://w3c.github.io/DOM-Parsing/#recording-the-namespace).alloc
    /// 
    /// 
    fn recording_namespace_information<'a, AttrIter>(&self, 
            map: &mut NamespacePrefixMap,
            local_prefixes_map: &mut LocalPrefixMap,
            attrs: AttrIter) -> Option<Namespace>
    where 
        AttrIter: Iterator<Item = AttrRef<'a>>
    {
        let mut default_namespace_attr_value = None;

        for attr in attrs {
            let attribute_namespace = attr.0.ns.clone();
            let attribute_prefix = attr.0.prefix.clone();

            if attribute_namespace == ns!(xmlns) {
                match attribute_prefix {
                    None => default_namespace_attr_value = Some(Namespace::from(attr.1)),
                    Some(prefix) => {
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
                        local_prefixes_map.insert(
                            prefix_definition,
                            namespace_definition.unwrap_or(ns!())
                        );
                    },
                }
            }
        }
        default_namespace_attr_value
    }
}

impl<Wr: Write> Serializer for XmlSerializer<Wr> {
    
    // TODO
    fn start_elem<'a, AttrIter>(&mut self, name: QualName, attrs: AttrIter, leaf_node: bool) -> io::Result<()>
    where
        AttrIter: Iterator<Item = AttrRef<'a>> {
        // 3.2.1.1 section 1
        if self.opts.require_well_formed {
            if name.local.contains(":") {
                return Err(Error::new(InvalidData, "Local name can't contain `:`."));
            }
            let mut first = true;
            for local_char in name.local.chars() {
                if first {
                    first = false;
                    if !is_name_start_char(local_char) {
                        return Err(Error::new(InvalidData, format!("Local name can't start with `{}`.", local_char)));
                    }
                } else {
                    if !is_name_char(local_char) {
                        return Err(Error::new(InvalidData, format!("Local name can't contain `{}`.", local_char)));
                    }
                }
            }
        }
        self.writer.write_all(b"<");
        let qualified_name = "";
        let skip_end_tag = false;
        let ignore_namespace_definition = false;
        let mut map = self.opts.prefix_map.clone();
        let mut local_prefixes_map = LocalPrefixMap::new();
        let local_default_namespace = self.recording_namespace_information(&mut map, &mut local_prefixes_map, attrs);

        /*
        let inherited_ns = self.opts.context_namespace.clone();
        let ns = name.ns;

        if inherited_ns == ns {

        }*/

        self.writer.write_all(b">")
    }


    // TODO
    fn end_elem(&mut self, name: QualName) -> io::Result<()> {
        Ok(())
    }

    // TODO Until DTD is fully implementeds
    fn write_doctype(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(b"<!DOCTYPE ")?;
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"!>")
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        self.write_escaped_text(text)
    }

    fn write_processing_instruction(&mut self, target: &str, data: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if "xml".eq_ignore_ascii_case(target) {
                return Err(Error::new(InvalidData, "Processing instruction target can't be equal to `xml`."));  
            }
            if target.contains(':') {
                return Err(Error::new(InvalidData, "Processing instruction target contains illegal characters `:`.")); 
            }
            if data.contains("?>") {
                return Err(Error::new(InvalidData, "Processing instruction data contains illegal characters `?>`."))
            }
        }
        self.writer.write_all(b"<?")?;
        self.write_escaped_text(target)?;
        self.writer.write_all(b" ")?;
        self.write_escaped_text(data)?;
        self.writer.write_all(b"?>")
    }

    fn write_comment(&mut self, text: &str) -> io::Result<()> {
        if self.opts.require_well_formed {
            if text.contains("--") {
                return Err(Error::new(InvalidData, "Comment contains double minus `--`")); 
            } else if text.ends_with("-") {
                return Err(Error::new(InvalidData, "Comment ends with minus"));
            }
        }
        self.writer.write_all(b"<!--")?;
        self.write_escaped_text(text)?;
        self.writer.write_all(b"-->")
    }
}
/*
#[derive(Debug)]
struct NamespaceMapStack(Vec<NamespaceMap>);

impl NamespaceMapStack {
    fn new() -> NamespaceMapStack {
        NamespaceMapStack(vec![])
    }

    fn push(&mut self, namespace: NamespaceMap) {
        self.0.push(namespace);
    }

    fn pop(&mut self) {
        self.0.pop();
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
fn write_to_buf_escaped<W: Write>(writer: &mut W, text: &str, attr_mode: bool) -> io::Result<()> {
    for c in text.chars() {
        match c {
            '&' => writer.write_all(b"&amp;"),
            '\'' if attr_mode => writer.write_all(b"&apos;"),
            '"' if attr_mode => writer.write_all(b"&quot;"),
            '<' if !attr_mode => writer.write_all(b"&lt;"),
            '>' if !attr_mode => writer.write_all(b"&gt;"),
            c => writer.write_fmt(format_args!("{}", c)),
        }?;
    }
    Ok(())
}

#[inline]
fn write_qual_name<W: Write>(writer: &mut W, name: &QualName) -> io::Result<()> {
    if let Some(ref prefix) = name.prefix {
        writer.write_all(&prefix.as_bytes())?;
        writer.write_all(b":")?;
        writer.write_all(&*name.local.as_bytes())?;
    } else {
        writer.write_all(&*name.local.as_bytes())?;
    }

    Ok(())
}

impl<Wr: Write> XmlSerializer<Wr> {
    /// Creates a new Serializier from a writer and given serialization options.
    pub fn new(writer: Wr) -> Self {
        XmlSerializer {
            writer: writer,
            namespace_stack: NamespaceMapStack::new(),
        }
    }

    #[inline(always)]
    fn qual_name(&mut self, name: &QualName) -> io::Result<()> {
        self.find_or_insert_ns(name);
        write_qual_name(&mut self.writer, name)
    }

    #[inline(always)]
    fn qual_attr_name(&mut self, name: &QualName) -> io::Result<()> {
        self.find_or_insert_ns(name);
        write_qual_name(&mut self.writer, name)
    }

    fn find_uri(&self, name: &QualName) -> bool {
        let mut found = false;
        for stack in self.namespace_stack.0.iter().rev() {
            if let Some(&Some(ref el)) = stack.get(&name.prefix) {
                found = *el == name.ns;
                break;
            }
        }
        found
    }

    fn find_or_insert_ns(&mut self, name: &QualName) {
        if name.prefix.is_some() || &*name.ns != "" {
            if !self.find_uri(name) {
                if let Some(last_ns) = self.namespace_stack.0.last_mut() {
                    last_ns.insert(name);
                }
            }
        }
    }
}

impl<Wr: Write> Serializer for XmlSerializer<Wr> {
    /// Serializes given start element into text. Start element contains
    /// qualified name and an attributes iterator.
    fn start_elem<'a, AttrIter>(&mut self, name: QualName, attrs: AttrIter) -> io::Result<()>
    where
        AttrIter: Iterator<Item = AttrRef<'a>>,
    {
        self.namespace_stack.push(NamespaceMap::empty());

        self.writer.write_all(b"<")?;
        self.qual_name(&name)?;
        if let Some(current_namespace) = self.namespace_stack.0.last() {
            for (prefix, url_opt) in current_namespace.get_scope_iter() {
                self.writer.write_all(b" xmlns")?;
                if let &Some(ref p) = prefix {
                    self.writer.write_all(b":")?;
                    self.writer.write_all(&*p.as_bytes())?;
                }

                self.writer.write_all(b"=\"")?;
                let url = if let &Some(ref a) = url_opt {
                    a.as_bytes()
                } else {
                    b""
                };
                self.writer.write_all(url)?;
                self.writer.write_all(b"\"")?;
            }
        }
        for (name, value) in attrs {
            self.writer.write_all(b" ")?;
            self.qual_attr_name(&name)?;
            self.writer.write_all(b"=\"")?;
            write_to_buf_escaped(&mut self.writer, value, true)?;
            self.writer.write_all(b"\"")?;
        }
        self.writer.write_all(b">")?;
        Ok(())
    }

    /// Serializes given end element into text.
    fn end_elem(&mut self, name: QualName) -> io::Result<()> {
        self.namespace_stack.pop();
        self.writer.write_all(b"</")?;
        self.qual_name(&name)?;
        self.writer.write_all(b">")
    }

    /// Serializes comment into text.
    fn write_comment(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(b"<!--")?;
        self.writer.write_all(text.as_bytes())?;
        self.writer.write_all(b"-->")
    }

    /// Serializes given doctype
    fn write_doctype(&mut self, name: &str) -> io::Result<()> {
        self.writer.write_all(b"<!DOCTYPE ")?;
        self.writer.write_all(name.as_bytes())?;
        self.writer.write_all(b">")
    }

    /// Serializes text for a node or an attributes.
    fn write_text(&mut self, text: &str) -> io::Result<()> {
        write_to_buf_escaped(&mut self.writer, text, false)
    }

    /// Serializes given processing instruction.
    fn write_processing_instruction(&mut self, target: &str, data: &str) -> io::Result<()> {
        self.writer.write_all(b"<?")?;
        self.writer.write_all(target.as_bytes())?;
        self.writer.write_all(b" ")?;
        self.writer.write_all(data.as_bytes())?;
        self.writer.write_all(b"?>")
    }
}
*/
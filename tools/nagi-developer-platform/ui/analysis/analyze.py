"""Bounded Qt Designer / GTK Builder / XAML inspection; UI content is never executed."""

import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys
import xml.etree.ElementTree as ET

MAX_BYTES = 65_536
MAX_ELEMENTS = 2_048
MAX_DEPTH = 32

# Recognition describes this parser's subset, not Nagi runtime support.
WIDGETS = frozenset({
    "QWidget", "QMainWindow", "QDialog", "QLabel", "QLineEdit",
    "QPushButton", "QCheckBox", "QRadioButton", "QComboBox",
})
LAYOUTS = frozenset({"QVBoxLayout", "QHBoxLayout", "QGridLayout", "QFormLayout"})
TAGS = frozenset({
    "ui", "class", "widget", "layout", "item", "property", "attribute",
    "string", "cstring", "stringlist", "bool", "number", "double", "enum",
    "set", "rect", "x", "y", "width", "height", "size", "sizepolicy",
    "horstretch", "verstretch", "spacer", "resources", "include", "connections",
    "connection", "sender", "signal", "receiver", "slot", "slots", "tabstops",
    "tabstop", "customwidgets", "customwidget", "extends", "header",
    "pixmap", "iconset", "normaloff", "normalon", "font", "family",
    "pointsize", "bold", "italic",
})


class AnalysisError(ValueError):
    """Errors expose a stable code without source content or absolute paths."""

    def __init__(self, code):
        self.code = code
        super().__init__(code)


def _text(element):
    return "".join(element.itertext()) if element is not None else ""


def _properties(element):
    return [
        {
            "name": prop.get("name", ""),
            "values": [
                {"kind": value.tag, "text": _text(value),
                 "attributes": dict(sorted(value.attrib.items()))}
                for value in prop
            ],
        }
        for prop in element
        if prop.tag in ("property", "attribute")
    ]


def _layout_items(element):
    return [
        {
            "attributes": dict(sorted(item.attrib.items())),
            "objects": [
                {
                    "kind": child.tag,
                    "attributes": dict(sorted(child.attrib.items())),
                    "properties": _properties(child),
                }
                for child in item
            ],
        }
        for item in element if item.tag == "item"
    ]


def _parse_xml(data):
    """Apply the same input/security bounds before selecting any format."""
    return _parse_xml_namespaces(data)[0]


def _parse_xml_namespaces(data):
    """Bounded parse returning (root, {element: [(prefix, uri), ...]}).

    ElementTree consumes xmlns declarations; they are recorded so XAML (whose
    meaning depends on them) can keep them as data. No namespace is resolved
    to code or fetched.
    """
    if len(data) > MAX_BYTES:
        raise AnalysisError("input_too_large")
    try:
        text = data.decode("utf-8-sig", errors="strict")
    except UnicodeDecodeError:
        raise AnalysisError("invalid_utf8") from None
    if "\x00" in text:
        raise AnalysisError("invalid_utf8")
    if re.search(r"<!\s*(?:DOCTYPE|ENTITY)\b", text, re.IGNORECASE):
        raise AnalysisError("declarations_forbidden")
    if re.search(r"<\?(?!xml(?:\s|\?>))", text):
        raise AnalysisError("processing_instruction_forbidden")

    parser = ET.XMLPullParser(events=("start", "end", "start-ns"))
    root = None
    count = depth = 0
    pending, declarations = [], {}
    try:
        for offset in range(0, len(text), 1024):
            parser.feed(text[offset:offset + 1024])
            for event, element in parser.read_events():
                if event == "start-ns":
                    pending.append((element[0] or "", element[1]))
                elif event == "start":
                    count += 1
                    depth += 1
                    if count > MAX_ELEMENTS:
                        raise AnalysisError("element_limit")
                    if depth > MAX_DEPTH:
                        raise AnalysisError("depth_limit")
                    if root is None:
                        root = element
                    if pending:
                        declarations[element] = pending
                        pending = []
                else:
                    depth -= 1
        parser.close()
    except ET.ParseError:
        raise AnalysisError("invalid_xml") from None
    return root, declarations


def analyze_bytes(data):
    """Select the format by its parsed root, without evaluating declarations."""
    root, declarations = _parse_xml_namespaces(data)
    if root is not None and root.tag == "interface":
        return _analyze_gtk(root)
    if root is not None and root.tag.startswith("{" + XAML_NS + "}"):
        return _analyze_xaml(root, declarations)
    if root is None or root.tag != "ui" or root.get("version") != "4.0":
        raise AnalysisError("unsupported_format")
    return _analyze_qt(root)


def _analyze_qt(root):
    if root.find("widget") is None:
        raise AnalysisError("missing_root_widget")

    widgets, layouts, unknown, bindings, resources = [], [], [], [], []
    names = set()
    stack = [(root, None, None)]
    while stack:
        element, widget_parent, layout_parent = stack.pop()
        owner = widget_parent
        if element.tag in ("widget", "layout"):
            name, kind = element.get("name"), element.get("class")
            if not name or not kind:
                raise AnalysisError("missing_object_identity")
            if name in names:
                raise AnalysisError("duplicate_object_name")
            names.add(name)
            item = {
                "name": name, "class": kind, "parent_widget": widget_parent,
                "parent_layout": layout_parent, "properties": _properties(element),
                "attributes": dict(sorted(element.attrib.items())),
            }
            if element.tag == "widget":
                item["recognized"] = kind in WIDGETS
                widgets.append(item)
                widget_parent, layout_parent, owner = name, None, name
            else:
                item["recognized"] = kind in LAYOUTS
                item["items"] = _layout_items(element)
                layouts.append(item)
                layout_parent = name
            if not item["recognized"]:
                unknown.append({"kind": element.tag, "name": name, "class": kind})
        if element.tag not in TAGS:
            unknown.append({"kind": "element", "tag": element.tag, "owner": owner})
        if "binding" in element.tag.lower():
            bindings.append({"owner": owner, "tag": element.tag, "text": _text(element)})
        if element.tag == "include":
            resources.append({"kind": "include", "value": element.get("location", _text(element))})
        if element.tag in ("pixmap", "iconset"):
            resources.append({"kind": element.tag, "value": _text(element)})
        stack.extend((child, widget_parent, layout_parent) for child in reversed(element))

    connections = []
    for connection in root.findall("./connections/connection"):
        fact = {key: connection.findtext(key, "") for key in
                ("sender", "signal", "receiver", "slot")}
        if any(not value for value in fact.values()):
            raise AnalysisError("incomplete_connection")
        fact["execution"] = "not_run"
        connections.append(fact)
    tab_order = [_text(item) for item in root.findall("./tabstops/tabstop")]
    widget_names = {item["name"] for item in widgets}
    unresolved = [
        {"kind": "tabstop", "name": name} for name in tab_order if name not in widget_names
    ]
    for fact in connections:
        for key in ("sender", "receiver"):
            if fact[key] not in widget_names:
                unresolved.append({"kind": key, "name": fact[key]})
    custom_widgets = [
        {
            "class": declaration.findtext("class", ""),
            "extends": declaration.findtext("extends", ""),
            "header": {
                "text": _text(declaration.find("header")),
                "attributes": dict(sorted(declaration.find("header").attrib.items()))
                if declaration.find("header") is not None else {},
            },
            "metadata": [
                {
                    "kind": child.tag, "text": _text(child),
                    "attributes": dict(sorted(child.attrib.items())),
                }
                for child in declaration
            ],
            "execution": "not_run",
        }
        for declaration in root.findall("./customwidgets/customwidget")
    ]
    return {
        "analysis_version": 1, "format": "qt-designer-ui",
        "form_class": root.findtext("class", ""), "widgets": widgets,
        "layouts": layouts, "connections": connections, "tab_order": tab_order,
        "declared_slots": [_text(item) for item in root.findall("./slots/slot")],
        "bindings": bindings, "resources": resources, "unknown": unknown,
        "custom_widgets": custom_widgets,
        "unresolved_references": unresolved,
        "limitations": [
            "Source structure only; no Qt or Nagi compatibility verdict.",
            "Resources, bindings and events are not fetched or executed.",
            "Rendering, input, focus, accessibility and localization are not verified.",
        ],
    }


# GTK containers are both objects in the hierarchy and layout owners. These
# allowlists recognize source vocabulary only, never Nagi runtime support.
GTK_LAYOUTS = frozenset({
    "GtkBox", "GtkVBox", "GtkHBox", "GtkGrid", "GtkTable", "GtkFixed",
    "GtkStack", "GtkOverlay", "GtkPaned",
})
GTK_WIDGETS = GTK_LAYOUTS | frozenset({
    "GtkWindow", "GtkApplicationWindow", "GtkDialog", "GtkLabel", "GtkEntry",
    "GtkButton", "GtkCheckButton", "GtkRadioButton", "GtkComboBox", "GtkSwitch",
    "GtkFrame", "GtkScrolledWindow", "GtkImage", "GtkTextView", "GtkListBox",
    "GtkSeparator", "GtkSpinButton", "GtkProgressBar",
})
GTK_TAGS = frozenset({
    "interface", "requires", "object", "child", "property", "packing", "layout",
    "signal", "accessibility", "relation", "state", "action", "binding",
    "lookup", "constant", "closure", "placeholder", "include",
})
# `template` is deliberately not listed: its class/parent declaration and body are
# not analyzed as widgets, so it stays reported as unknown (with evidence) rather
# than looking recognized.


def _gtk_evidence(element):
    """Keep nested expression/unknown syntax as data, within the XML bounds."""
    return {
        "kind": element.tag, "attributes": dict(sorted(element.attrib.items())),
        "text": element.text or "",
        "children": [_gtk_evidence(child) for child in element],
        "tail": element.tail or "",
    }


def _gtk_properties(element):
    if element is None:
        return []
    return [
        {
            "name": prop.get("name", "") if prop.tag == "property" else prop.tag,
            "values": [{"kind": "text", "text": prop.text or "",
                        "attributes": dict(sorted(prop.attrib.items()))}]
                      + [_gtk_evidence(child) for child in prop],
        }
        for prop in element if prop.tag in ("property", "accessibility", "layout")
    ]


def _gtk_layout_items(element, identities):
    return [
        {
            "attributes": dict(sorted(child.attrib.items())),
            "packing": _gtk_properties(child.find("packing")),
            "objects": [
                {"kind": obj.tag, "name": identities[obj],
                 "attributes": dict(sorted(obj.attrib.items())),
                 "properties": _gtk_properties(obj)}
                for obj in child if obj.tag == "object"
            ],
        }
        for child in element if child.tag == "child"
    ]


def _analyze_gtk(root):
    if root.find("object") is None:
        raise AnalysisError("missing_root_widget")

    # Builder permits anonymous objects. Reserve every explicit ID first so
    # generated report identities cannot alias a source ID or a later object.
    names, identities = set(), {}
    objects = list(root.iter("object"))
    for obj in objects:
        name = obj.get("id")
        if not obj.get("class") or name == "":
            raise AnalysisError("missing_object_identity")
        if name is not None:
            if name in names:
                raise AnalysisError("duplicate_object_name")
            names.add(name)
            identities[obj] = name
    for index, obj in enumerate(objects, 1):
        if obj not in identities:
            name = "@gtk-object-" + str(index)
            while name in names:
                name += "_"
            identities[obj] = name
            names.add(name)

    widgets, layouts, connections, bindings = [], [], [], []
    resources, unknown, unresolved = [], [], []
    # References resolve against source IDs, not generated report identities.
    source_ids = {obj.get("id") for obj in objects if obj.get("id")}

    def reference(kind, name):
        if name and name not in source_ids:
            unresolved.append({"kind": kind, "name": name})

    stack = [(root, None, None, False)]
    while stack:
        element, widget_parent, layout_parent, inside_unknown = stack.pop()
        owner = widget_parent
        attributes = dict(sorted(element.attrib.items()))
        if element.tag == "object":
            name, kind = identities[element], element.get("class")
            item = {
                "name": name, "class": kind, "parent_widget": widget_parent,
                "parent_layout": layout_parent, "properties": _gtk_properties(element),
                "attributes": attributes, "recognized": kind in GTK_WIDGETS,
                "children": _gtk_layout_items(element, identities),
            }
            widgets.append(item)
            if not item["recognized"]:
                unknown.append({"kind": "widget", "name": name, "class": kind})
            if kind in GTK_LAYOUTS:
                layouts.append({
                    "name": name, "class": kind, "parent_widget": name,
                    "parent_layout": layout_parent, "properties": item["properties"],
                    "attributes": attributes, "recognized": True,
                    "items": item["children"],
                })
            widget_parent, owner = name, name
            layout_parent = name if kind in GTK_LAYOUTS else None
        is_unknown = element.tag not in GTK_TAGS
        if is_unknown:
            entry = {"kind": "element", "tag": element.tag, "owner": owner}
            if inside_unknown:
                # The outermost unknown element already carries this subtree as
                # `content`; repeating it per descendant would copy it O(depth) times.
                entry["nested_in_unknown"] = True
            else:
                entry["content"] = _gtk_evidence(element)
            unknown.append(entry)
        if element.tag == "signal":
            signal, handler = element.get("name", ""), element.get("handler", "")
            if owner is None or not signal or not handler:
                raise AnalysisError("incomplete_connection")
            receiver = element.get("object", "")
            connections.append({
                "sender": owner, "signal": signal, "receiver": receiver,
                "slot": handler, "attributes": attributes, "execution": "not_run",
            })
            reference("receiver", receiver)
        if "binding" in element.tag.lower() or (
                element.tag == "property" and any(
                    key.startswith("bind-") for key in attributes)):
            bindings.append({"owner": owner, "tag": element.tag,
                             "text": _text(element), "content": _gtk_evidence(element),
                             "execution": "not_run"})
            reference("binding_source", element.get("bind-source", ""))
        if element.tag == "property":
            # Property objects are owned values, not packed layout children.
            layout_parent = None
            if element.get("name") in ("file", "filename", "resource", "pixbuf"):
                resources.append({"kind": element.get("name"), "value": _text(element),
                                  "owner": owner, "attributes": attributes})
        if element.tag == "requires":
            resources.append({"kind": "requires", "value": element.get("lib", ""),
                              "attributes": attributes})
        if element.tag == "include":
            resources.append({"kind": "include", "value": element.get("filename", ""),
                              "attributes": attributes})
        stack.extend((child, widget_parent, layout_parent, inside_unknown or is_unknown)
                     for child in reversed(element))

    return {
        "analysis_version": 1, "format": "gtk-builder-ui", "form_class": "",
        "attributes": dict(sorted(root.attrib.items())),
        "widgets": widgets, "layouts": layouts, "connections": connections,
        "tab_order": [], "declared_slots": [], "bindings": bindings,
        "resources": resources, "unknown": unknown, "custom_widgets": [],
        "unresolved_references": unresolved,
        "limitations": [
            "Source structure only; no GTK or Nagi compatibility verdict.",
            "Resources, bindings and events are not fetched or executed.",
            "Rendering, input, focus, accessibility and localization are not verified.",
            "GTK class/property types and general object references are not validated.",
        ],
    }


# ---------------------------------------------------------------------------
# XAML (WPF / WinUI) -- NDP-06d. Everything below is PROVISIONAL vocabulary
# data: replacing a table changes recognition only, never what is executed.
# x:Class / code-behind, markup extensions, converters, resource dictionaries
# and templates are never run, loaded or resolved; they stay data or unknown.
# ---------------------------------------------------------------------------
XAML_NS = "http://schemas.microsoft.com/winfx/2006/xaml/presentation"
XAML_LANG_NS = "http://schemas.microsoft.com/winfx/2006/xaml"
XML_NS = "http://www.w3.org/XML/1998/namespace"
XAML_META_NAMESPACES = frozenset({
    XAML_LANG_NS,
    "http://schemas.openxmlformats.org/markup-compatibility/2006",
    "http://schemas.microsoft.com/expression/blend/2008",
})
XAML_LAYOUTS = frozenset({
    "Grid", "StackPanel", "WrapPanel", "DockPanel", "Canvas", "UniformGrid",
    "RelativePanel", "VariableSizedWrapGrid",
})
XAML_CONTROLS = XAML_LAYOUTS | frozenset({
    "Window", "Page", "UserControl", "ContentDialog", "Button", "ToggleButton",
    "TextBlock", "TextBox", "PasswordBox", "Label", "CheckBox", "RadioButton",
    "ComboBox", "ToggleSwitch", "Slider", "ProgressBar", "ListBox", "ListView",
    "Image", "Border", "ScrollViewer", "GroupBox", "Expander", "Separator",
    "TabControl", "TabItem", "Menu", "MenuItem", "ToolBar", "StatusBar", "Frame",
    "ContentControl", "ItemsControl", "DataGrid", "TreeView", "RichTextBox",
})
# Non-visual data objects: kept as property-value evidence, never as widgets.
# Templates (ControlTemplate/DataTemplate/...) are deliberately absent so they
# are reported as unknown with their body, like GTK `template`.
XAML_VALUE_TYPES = frozenset({
    "Setter", "Style", "ResourceDictionary", "SolidColorBrush", "LinearGradientBrush",
    "RadialGradientBrush", "ImageBrush", "GradientStop", "RowDefinition",
    "ColumnDefinition", "Binding", "MultiBinding", "PriorityBinding", "RelativeSource",
    "Run", "Span", "Bold", "Italic", "Underline", "LineBreak", "Hyperlink",
    "Trigger", "DataTrigger", "EventTrigger", "EventSetter", "Condition",
    "TranslateTransform", "ScaleTransform", "RotateTransform", "TransformGroup",
    "Storyboard",
})
# Attribute names treated as event handler declarations (name only; no type info).
XAML_EVENTS = frozenset({
    "Click", "DoubleClick", "Tapped", "DoubleTapped", "RightTapped", "Holding",
    "Loaded", "Unloaded", "Initialized", "Closed", "Closing", "Activated",
    "Deactivated", "TextChanged", "SelectionChanged", "Checked", "Unchecked",
    "Indeterminate", "Toggled", "ValueChanged", "SizeChanged", "GotFocus",
    "LostFocus", "KeyDown", "KeyUp", "PreviewKeyDown", "PreviewKeyUp",
    "MouseDown", "MouseUp", "MouseMove", "MouseEnter", "MouseLeave",
    "MouseLeftButtonDown", "MouseLeftButtonUp", "PointerPressed",
    "PointerReleased", "PointerEntered", "PointerExited", "Drop", "DragOver",
    "TextInput", "PreviewTextInput", "Navigated", "Opened",
})
# Name of the property that receives direct text, per class; default "Content".
XAML_CONTENT_PROPERTY = {
    "TextBlock": "Text", "TextBox": "Text", "Run": "Text", "Span": "Inlines",
    "Bold": "Inlines", "Italic": "Inlines", "Underline": "Inlines",
    "Hyperlink": "Inlines", "PasswordBox": "Password",
}
XAML_BINDING_EXTENSIONS = frozenset({"Binding", "x:Bind", "TemplateBinding"})
XAML_RESOURCE_EXTENSIONS = frozenset({"StaticResource", "DynamicResource", "ThemeResource"})
# Understood as inert syntax: kept as property evidence, nothing extra reported.
XAML_INERT_EXTENSIONS = frozenset({"x:Null", "x:Type", "x:Static", "x:Reference", "RelativeSource"})
# Attributes whose value names an external URI/file: recorded, never fetched.
XAML_RESOURCE_ATTRIBUTES = frozenset({"Source", "ImageSource", "NavigateUri", "UriSource"})
MARKUP_MAX_DEPTH = 8
_TAB_INDEX = re.compile(r"-?[0-9]{1,9}")


def _xaml_split(tag):
    if tag.startswith("{"):
        uri, _, local = tag[1:].partition("}")
        return uri, local
    return "", tag


def _xaml_name(key):
    """Display name of an attribute: `x:Name`, `xml:lang`, or `{uri}name` as-is."""
    uri, local = _xaml_split(key)
    if not uri:
        return local
    if uri == XAML_LANG_NS:
        return "x:" + local
    if uri == XML_NS:
        return "xml:" + local
    return key


def _xaml_attributes(element):
    return {_xaml_name(key): value for key, value in sorted(element.attrib.items())}


def _xaml_evidence(element):
    """Keep any subtree as data (same keys as the GTK evidence shape)."""
    uri, local = _xaml_split(element.tag)
    return {
        "kind": local if uri == XAML_NS else element.tag,
        "attributes": _xaml_attributes(element), "text": element.text or "",
        "children": [_xaml_evidence(child) for child in element],
        "tail": element.tail or "",
    }


def _markup_split(text, separators):
    """Split at top level (outside braces/quotes). Returns None when unbalanced."""
    parts, current, depth, quote, index = [], [], 0, "", 0
    while index < len(text):
        char = text[index]
        if char == "\\" and index + 1 < len(text):
            current.append(text[index:index + 2])
            index += 2
            continue
        if quote:
            quote = "" if char == quote else quote
        elif char in "'\"":
            quote = char
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth < 0:
                return None
        elif char in separators and depth == 0:
            parts.append("".join(current))
            current = []
            index += 1
            continue
        current.append(char)
        index += 1
    if depth or quote:
        return None
    parts.append("".join(current))
    return parts


def _markup_parse(text, depth=0):
    """Parse `{Name arg, key=value}` structurally. Returns None if not parseable.

    Pure string processing: nothing named in the extension is looked up or run.
    """
    text = text.strip()
    if depth > MARKUP_MAX_DEPTH or len(text) < 2 or text[0] != "{" or text[-1] != "}":
        return None
    inner = text[1:-1].strip()
    head = re.match(r"[^\s,=]*", inner).group(0)
    rest = inner[len(head):].strip()
    if not head:
        return None
    pieces = _markup_split(rest, ",") if rest else []
    if pieces is None:
        return None
    arguments = []
    for position, raw in enumerate(pieces):
        raw = raw.strip()
        pair = _markup_split(raw, "=")
        if pair is None:
            return None
        key, value = ("", raw) if len(pair) == 1 else (pair[0].strip(), "=".join(pair[1:]).strip())
        nested = None
        if value.startswith("{") and not value.startswith("{}"):
            nested = _markup_parse(value, depth + 1)
            if nested is None:
                return None
        elif len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        arguments.append({"name": key, "position": position, "value": value,
                          "nested": nested})
    return {"extension": head, "arguments": arguments, "raw": text}


def _markup_evidence(parsed):
    return {
        "kind": "markup_extension", "attributes": {"extension": parsed["extension"]},
        "text": parsed["raw"],
        "children": [
            {"kind": "argument",
             "attributes": {"name": arg["name"], "position": str(arg["position"])},
             "text": arg["value"] if arg["nested"] is None else "",
             "children": [] if arg["nested"] is None else [_markup_evidence(arg["nested"])],
             "tail": ""}
            for arg in parsed["arguments"]
        ],
        "tail": "",
    }


def _is_markup(value):
    return value.startswith("{") and not value.startswith("{}")


def _analyze_xaml(root, declarations):
    elements = list(root.iter())
    XNAME, XKEY = "{" + XAML_LANG_NS + "}Name", "{" + XAML_LANG_NS + "}Key"

    def kind_of(element):
        uri, local = _xaml_split(element.tag)
        if "." in local:
            before, _, after = local.partition(".")
            return "property" if before and after and "." not in after else "unknown"
        if uri == XAML_NS:
            if local in XAML_CONTROLS:
                return "widget"
            return "value" if local in XAML_VALUE_TYPES else "unknown"
        if not uri or uri in XAML_META_NAMESPACES or element.get(XKEY) is not None:
            return "unknown"
        return "custom"

    kinds = {element: kind_of(element) for element in elements}

    def explicit_name(element):
        names = {element.get(XNAME), element.get("Name")} - {None}
        if len(names) > 1:
            raise AnalysisError("conflicting_object_name")
        return names.pop() if names else None

    # Reserve every explicit name first so generated identities cannot alias it.
    names, identities, source_names = set(), {}, set()
    for element in elements:
        name = explicit_name(element)
        if name is None:
            continue
        if name == "":
            raise AnalysisError("missing_object_identity")
        if name in names:
            raise AnalysisError("duplicate_object_name")
        names.add(name)
        source_names.add(name)
        if kinds[element] in ("widget", "custom"):
            identities[element] = name
    widget_elements = [e for e in elements if kinds[e] in ("widget", "custom")]
    if not widget_elements:
        raise AnalysisError("missing_root_widget")
    for index, element in enumerate(widget_elements, 1):
        if element not in identities:
            name = "@xaml-object-" + str(index)
            while name in names:
                name += "_"
            identities[element] = name
            names.add(name)
    resource_keys = {e.get(XKEY) for e in elements if e.get(XKEY) is not None}

    def class_of(element):
        uri, local = _xaml_split(element.tag)
        return local if uri == XAML_NS else element.tag

    def content_name(element):
        return XAML_CONTENT_PROPERTY.get(_xaml_split(element.tag)[1], "Content")

    def property_name(owner, child):
        before, _, after = _xaml_split(child.tag)[1].partition(".")
        return after if before == _xaml_split(owner.tag)[1] else _xaml_split(child.tag)[1]

    def is_event(name):
        return name.rsplit(".", 1)[-1] in XAML_EVENTS and not name.startswith("{")

    property_cache = {}

    def properties_of(element):
        if element in property_cache:
            return property_cache[element]
        found = []
        for name, value in _xaml_attributes(element).items():
            if name in ("x:Name", "Name") or is_event(name):
                continue
            values = [{"kind": "text", "text": value, "attributes": {}}]
            if _is_markup(value):
                parsed = _markup_parse(value)
                if parsed is not None:
                    values.append(_markup_evidence(parsed))
            found.append({"name": name, "values": values})
        for child in element:
            if kinds[child] == "property":
                found.append({
                    "name": property_name(element, child),
                    "values": [{"kind": "text", "text": child.text or "",
                                "attributes": _xaml_attributes(child)}]
                              + [_xaml_evidence(grand) for grand in child],
                })
        text = (element.text or "") + "".join(child.tail or "" for child in element)
        inline = [_xaml_evidence(child) for child in element
                  if kinds[child] in ("value", "unknown")]
        if text.strip() or inline:
            found.append({"name": content_name(element),
                          "values": [{"kind": "text", "text": text, "attributes": {}}] + inline})
        property_cache[element] = found
        return found

    def placements(element):
        items = []
        for child in element:
            if kinds[child] in ("widget", "custom"):
                holders = [(child, "")]
            elif kinds[child] == "property":
                holders = [(grand, property_name(element, child)) for grand in child
                           if kinds[grand] in ("widget", "custom")]
            else:
                holders = []
            for obj, via in holders:
                attributes = _xaml_attributes(obj)
                items.append({
                    "attributes": {k: v for k, v in attributes.items() if "." in k},
                    "property": via,
                    "objects": [{"kind": "object", "name": identities[obj],
                                 "attributes": attributes,
                                 "properties": properties_of(obj)}],
                })
        return items

    widgets, layouts, connections, bindings = [], [], [], []
    resources, unknown, unresolved, custom_widgets = [], [], [], []
    tab_stops = []

    def reference(kind, name):
        known = resource_keys if kind == "resource_key" else source_names
        if name and name not in known:
            unresolved.append({"kind": kind, "name": name})

    def scan_attributes(element, owner):
        for name, value in _xaml_attributes(element).items():
            if is_event(name):
                if not value:
                    raise AnalysisError("incomplete_connection")
                if owner is None:
                    raise AnalysisError("incomplete_connection")
                connections.append({
                    "sender": owner, "signal": name.rsplit(".", 1)[-1], "receiver": "",
                    "slot": value, "attributes": {"attribute": name},
                    "execution": "not_run"})
            if name in XAML_RESOURCE_ATTRIBUTES and value and not _is_markup(value):
                resources.append({"kind": name, "value": value, "owner": owner,
                                  "attributes": {"attribute": name}})
            if name == "ElementName":
                reference("binding_source", value)
            if not _is_markup(value):
                continue
            parsed = _markup_parse(value)
            if parsed is None:
                unknown.append({"kind": "markup_extension", "extension": "", "owner": owner,
                                "property": name, "text": value, "parse": "unparseable"})
                continue
            classify_markup(parsed, owner, name)

    def classify_markup(parsed, owner, name):
        """Report one extension and, recursively, the extensions nested in its arguments."""
        extension, arguments, value = parsed["extension"], parsed["arguments"], parsed["raw"]
        if extension in XAML_BINDING_EXTENSIONS:
            bindings.append({"owner": owner, "tag": extension, "property": name,
                             "text": value, "content": _markup_evidence(parsed),
                             "execution": "not_run"})
            for argument in arguments:
                if argument["name"] == "ElementName":
                    reference("binding_source", argument["value"])
        elif extension in XAML_RESOURCE_EXTENSIONS:
            key = next((a["value"] for a in arguments if a["name"] in ("", "ResourceKey")), "")
            resources.append({"kind": "resource_reference", "value": key, "owner": owner,
                              "attributes": {"extension": extension, "property": name}})
            reference("resource_key", key)
        elif extension == "x:Reference":
            reference("binding_source", next((a["value"] for a in arguments), ""))
        elif extension not in XAML_INERT_EXTENSIONS:
            unknown.append({"kind": "markup_extension", "extension": extension,
                            "owner": owner, "property": name, "text": value})
        for argument in arguments:
            if argument["nested"] is not None:
                classify_markup(argument["nested"], owner, name)

    stack = [(root, None, None, False)]
    while stack:
        element, widget_parent, layout_parent, inside_unknown = stack.pop()
        kind, owner = kinds[element], widget_parent
        local = _xaml_split(element.tag)[1]
        attributes = _xaml_attributes(element)
        if kind in ("widget", "custom"):
            name, cls = identities[element], class_of(element)
            recognized = kind == "widget"
            item = {
                "name": name, "class": cls, "parent_widget": widget_parent,
                "parent_layout": layout_parent, "properties": properties_of(element),
                "attributes": attributes, "recognized": recognized,
                "children": placements(element),
            }
            widgets.append(item)
            if not recognized:
                unknown.append({"kind": "widget", "name": name, "class": cls})
                uri = _xaml_split(element.tag)[0]
                custom_widgets.append({
                    "class": local, "extends": "", "header": {"text": "", "attributes": {}},
                    "metadata": [{"kind": "xml-namespace", "text": uri, "attributes": {}}],
                    "execution": "not_run"})
            if cls in XAML_LAYOUTS:
                layouts.append({
                    "name": name, "class": cls, "parent_widget": name,
                    "parent_layout": layout_parent, "properties": item["properties"],
                    "attributes": attributes, "recognized": True, "items": item["children"]})
            widget_parent = owner = name
            layout_parent = name if cls in XAML_LAYOUTS else None
            if _TAB_INDEX.fullmatch(attributes.get("TabIndex", "")):
                tab_stops.append((int(attributes["TabIndex"]), len(tab_stops), name))
        is_unknown = kind == "unknown"
        if is_unknown:
            entry = {"kind": "element", "tag": class_of(element), "owner": owner}
            if inside_unknown:
                entry["nested_in_unknown"] = True
            else:
                entry["content"] = _xaml_evidence(element)
            unknown.append(entry)
        if kind == "property":
            layout_parent = None
        scan_attributes(element, owner)
        if "binding" in local.lower() and kind in ("value", "unknown"):
            bindings.append({"owner": owner, "tag": local, "property": "",
                             "text": "".join(element.itertext()),
                             "content": _xaml_evidence(element), "execution": "not_run"})
        if local == "EventSetter":
            event, handler = element.get("Event", ""), element.get("Handler", "")
            if owner is None or not event or not handler:
                raise AnalysisError("incomplete_connection")
            connections.append({"sender": owner, "signal": event, "receiver": "",
                                "slot": handler, "attributes": attributes,
                                "execution": "not_run"})
        key = element.get(XKEY)
        if key is not None:
            resources.append({"kind": "resource_definition", "value": key, "owner": owner,
                              "attributes": attributes})
        stack.extend((child, widget_parent, layout_parent, inside_unknown or is_unknown)
                     for child in reversed(element))

    tab_stops.sort()
    namespaces = [{"prefix": prefix, "uri": uri}
                  for element in elements for prefix, uri in declarations.get(element, [])]
    return {
        "analysis_version": 1, "format": "xaml-ui",
        "form_class": root.get("{" + XAML_LANG_NS + "}Class", ""),
        "attributes": _xaml_attributes(root), "namespaces": namespaces,
        "dialect_hints": sorted({
            item["uri"].split(":", 1)[0] for item in namespaces
            if item["uri"].startswith(("clr-namespace:", "using:"))}),
        "widgets": widgets, "layouts": layouts, "connections": connections,
        "tab_order": [name for _, _, name in tab_stops], "declared_slots": [],
        "bindings": bindings, "resources": resources, "unknown": unknown,
        "custom_widgets": custom_widgets, "unresolved_references": unresolved,
        "limitations": [
            "Source structure only; no WPF/WinUI or Nagi compatibility verdict.",
            "x:Class/code-behind, markup extensions, converters, resource dictionaries"
            " and templates are not loaded, resolved or executed.",
            "Rendering, input, focus, accessibility and localization are not verified.",
            "Element/attribute vocabulary is a provisional table; types are not resolved.",
        ],
    }


def analyze_file(relative_path, allowed_root):
    """Open one relative regular file through directory handles, without links."""
    path = Path(relative_path)
    if path.is_absolute() or not path.parts or any(part in (".", "..") for part in path.parts):
        raise AnalysisError("unsafe_path")
    if not all(hasattr(os, flag) for flag in ("O_DIRECTORY", "O_NOFOLLOW", "O_NONBLOCK")):
        raise AnalysisError("unsupported_host")
    descriptor = None
    try:
        descriptor = os.open(allowed_root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        for part in path.parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        file_descriptor = os.open(
            path.parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
            dir_fd=descriptor,
        )
        with os.fdopen(file_descriptor, "rb") as source:
            if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                raise AnalysisError("not_regular_file")
            data = source.read(MAX_BYTES + 1)
    except (OSError, ValueError) as error:
        if isinstance(error, AnalysisError):
            raise
        raise AnalysisError("input_unavailable") from None
    finally:
        if descriptor is not None:
            os.close(descriptor)
    result = analyze_bytes(data)
    result["source"] = path.as_posix()
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, help="Explicit allowed input directory")
    parser.add_argument("input", help="One file relative to --root")
    args = parser.parse_args(argv)
    try:
        result = analyze_file(args.input, args.root)
    except AnalysisError as error:
        print(json.dumps({"error": error.code}), file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

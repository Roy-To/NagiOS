"""Bounded Qt Designer / GTK Builder inspection; UI content is never executed."""

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
    """Apply the same input/security bounds before selecting either format."""
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

    parser = ET.XMLPullParser(events=("start", "end"))
    root = None
    count = depth = 0
    try:
        for offset in range(0, len(text), 1024):
            parser.feed(text[offset:offset + 1024])
            for event, element in parser.read_events():
                if event == "start":
                    count += 1
                    depth += 1
                    if count > MAX_ELEMENTS:
                        raise AnalysisError("element_limit")
                    if depth > MAX_DEPTH:
                        raise AnalysisError("depth_limit")
                    if root is None:
                        root = element
                else:
                    depth -= 1
        parser.close()
    except ET.ParseError:
        raise AnalysisError("invalid_xml") from None
    return root


def analyze_bytes(data):
    """Select the format by its parsed root, without evaluating declarations."""
    root = _parse_xml(data)
    if root is not None and root.tag == "interface":
        return _analyze_gtk(root)
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

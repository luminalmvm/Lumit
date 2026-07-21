// The Timeline's layer-search filter, ported from the egui top row's search box
// (crates/lumit-ui/src/shell/timeline/panel.rs: the `timeline_layer_search`
// filter): a case-insensitive substring match on the layer name, with an empty
// or whitespace-only query matching everything. Pure so the filter is
// unit-tested directly.

/// Whether a layer named [name] passes the [query] filter (case-insensitive
/// substring; an empty/blank query passes everything).
bool layerMatchesSearch(String name, String query) {
  final q = query.trim().toLowerCase();
  if (q.isEmpty) return true;
  return name.toLowerCase().contains(q);
}

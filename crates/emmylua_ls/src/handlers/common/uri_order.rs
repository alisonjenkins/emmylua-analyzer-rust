use emmylua_code_analysis::{FileId, SemanticDatabase};

/// Sort key that orders files by uri. File ids follow load and didOpen order, which differs between
/// runs, so answers that list several files must not be ordered by them.
pub(crate) fn uri_order_key(db: &SemanticDatabase, file_id: FileId) -> (Option<String>, u32) {
    let uri = db
        .document(file_id)
        .and_then(|document| document.get_uri())
        .map(|uri| uri.to_string());
    (uri, file_id.id)
}

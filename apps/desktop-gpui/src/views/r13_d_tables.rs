//! Table copy uses the renderer's semantic cells, not Markdown delimiters.
use gpui_kit::component::text::TableData;

pub(super) fn csv(table: &TableData) -> String {
    std::iter::once(&table.headers)
        .chain(table.rows.iter())
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.contains([',', '"', '\r', '\n']) {
                        format!("\"{}\"", cell.replace('"', "\"\""))
                    } else {
                        cell.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d_table_csv_quotes_semantic_cells_and_retains_ragged_rows() {
        let table = TableData {
            headers: vec!["Name".into(), "Value".into()],
            rows: vec![vec!["a,b".into(), "a\"b\nc".into()], vec!["tail".into()]],
            ..Default::default()
        };
        assert_eq!(csv(&table), "Name,Value\r\n\"a,b\",\"a\"\"b\nc\"\r\ntail");
    }
}

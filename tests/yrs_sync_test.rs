use yrs::{Doc, GetString, ReadTxn, Text, TextRef, Transact, updates::decoder::Decode};
use similar::{ChangeTag, TextDiff};

/// Same sync_to_yrs as in editor.rs
fn sync_to_yrs(text: &TextRef, doc: &Doc, old: &str, new: &str) {
    if old == new {
        return;
    }

    let line_diff = TextDiff::from_lines(old, new);
    let mut txn = doc.transact_mut();
    let mut pos: u32 = 0;

    for change in line_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                pos += change.value().len() as u32;
            }
            ChangeTag::Delete => {
                let len = change.value().len() as u32;
                text.remove_range(&mut txn, pos, len);
            }
            ChangeTag::Insert => {
                let value = change.value();
                text.insert(&mut txn, pos, value);
                pos += value.len() as u32;
            }
        }
    }
}

fn get_text(text: &TextRef, doc: &Doc) -> String {
    let txn = doc.transact();
    text.get_string(&txn)
}

fn encode_state(doc: &Doc) -> Vec<u8> {
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

fn apply_state(doc: &Doc, state: &[u8]) {
    let update = yrs::Update::decode_v1(state).unwrap();
    let mut txn = doc.transact_mut();
    txn.apply_update(update).unwrap();
}

#[test]
fn test_basic_insert() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello world\n");
    }

    let old = get_text(&text, &doc);
    assert_eq!(old, "hello world\n");

    // Simulate user adding a line
    let new_content = "hello world\nnew line here\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "hello world\nnew line here\n", "Insert line failed");
    println!("PASS: basic insert");
}

#[test]
fn test_delete_line() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "line one\nline two\nline three\n");
    }

    let old = get_text(&text, &doc);

    // Delete middle line
    let new_content = "line one\nline three\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "line one\nline three\n", "Delete line failed");
    println!("PASS: delete line");
}

#[test]
fn test_delete_multiple_lines() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    let initial = "line 1\nline 2\nline 3\nline 4\nline 5\n";
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, initial);
    }

    let old = get_text(&text, &doc);

    // Delete lines 2, 3, 4
    let new_content = "line 1\nline 5\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "line 1\nline 5\n", "Delete multiple lines failed");
    println!("PASS: delete multiple lines");
}

#[test]
fn test_newline_insert() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello world\n");
    }

    let old = get_text(&text, &doc);

    // User presses Enter in the middle of "hello world"
    let new_content = "hello\n world\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "hello\n world\n", "Newline insert failed");
    println!("PASS: newline insert");
}

#[test]
fn test_newline_delete() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello\nworld\n");
    }

    let old = get_text(&text, &doc);

    // User joins lines (backspace at start of "world")
    let new_content = "helloworld\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "helloworld\n", "Newline delete failed");
    println!("PASS: newline delete");
}

#[test]
fn test_empty_to_content() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "\n");
    }

    let old = get_text(&text, &doc);

    let new_content = "some content\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "some content\n", "Empty to content failed");
    println!("PASS: empty to content");
}

#[test]
fn test_content_to_empty() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "some content\nmore stuff\n");
    }

    let old = get_text(&text, &doc);

    let new_content = "\n";
    sync_to_yrs(&text, &doc, &old, new_content);

    let result = get_text(&text, &doc);
    assert_eq!(result, "\n", "Content to empty failed");
    println!("PASS: content to empty");
}

#[test]
fn test_incremental_edits() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello\n");
    }

    let mut last = get_text(&text, &doc);

    // Simulate multiple keystrokes
    let edits = vec![
        "hello\nw\n",
        "hello\nwo\n",
        "hello\nwor\n",
        "hello\nworl\n",
        "hello\nworld\n",
    ];

    for edit in &edits {
        sync_to_yrs(&text, &doc, &last, edit);
        last = get_text(&text, &doc);
        assert_eq!(&last, edit, "Incremental edit mismatch at '{}'", edit);
    }
    println!("PASS: incremental edits");
}

#[test]
fn test_two_user_merge() {
    // User A creates doc
    let doc_a = Doc::new();
    let text_a = doc_a.get_or_insert_text("content");
    {
        let mut txn = doc_a.transact_mut();
        text_a.insert(&mut txn, 0, "line 1\nline 2\nline 3\n");
    }

    // Push A's state to "server"
    let server_state = encode_state(&doc_a);

    // User B opens doc, pulls A's state
    let doc_b = Doc::new();
    let text_b = doc_b.get_or_insert_text("content");
    apply_state(&doc_b, &server_state);

    let b_content = get_text(&text_b, &doc_b);
    assert_eq!(b_content, "line 1\nline 2\nline 3\n", "B should have A's content");

    // User A edits: changes line 1
    let old_a = get_text(&text_a, &doc_a);
    sync_to_yrs(&text_a, &doc_a, &old_a, "line 1 EDITED BY A\nline 2\nline 3\n");

    // User B edits: changes line 3
    let old_b = get_text(&text_b, &doc_b);
    sync_to_yrs(&text_b, &doc_b, &old_b, "line 1\nline 2\nline 3 EDITED BY B\n");

    // A pushes state
    let state_a = encode_state(&doc_a);

    // B pushes state
    let state_b = encode_state(&doc_b);

    // A pulls B's state and merges
    apply_state(&doc_a, &state_b);
    let merged_a = get_text(&text_a, &doc_a);

    // B pulls A's state and merges
    apply_state(&doc_b, &state_a);
    let merged_b = get_text(&text_b, &doc_b);

    println!("Merged at A: {:?}", merged_a);
    println!("Merged at B: {:?}", merged_b);

    // Both should converge to the same content
    assert_eq!(merged_a, merged_b, "A and B should converge");

    // Both edits should be present
    assert!(merged_a.contains("EDITED BY A"), "A's edit should be in merged");
    assert!(merged_a.contains("EDITED BY B"), "B's edit should be in merged");

    println!("PASS: two user merge");
}

#[test]
fn test_two_user_delete_lines() {
    // User A creates doc
    let doc_a = Doc::new();
    let text_a = doc_a.get_or_insert_text("content");
    {
        let mut txn = doc_a.transact_mut();
        text_a.insert(&mut txn, 0, "keep\ndelete me\nalso keep\n");
    }

    let server_state = encode_state(&doc_a);

    // User B pulls
    let doc_b = Doc::new();
    let text_b = doc_b.get_or_insert_text("content");
    apply_state(&doc_b, &server_state);

    // User A deletes middle line
    let old_a = get_text(&text_a, &doc_a);
    sync_to_yrs(&text_a, &doc_a, &old_a, "keep\nalso keep\n");
    let after_a = get_text(&text_a, &doc_a);
    assert_eq!(after_a, "keep\nalso keep\n", "A's delete didn't work locally");

    // A pushes
    let state_a = encode_state(&doc_a);

    // B pulls A's state
    apply_state(&doc_b, &state_a);
    let after_b = get_text(&text_b, &doc_b);
    assert_eq!(after_b, "keep\nalso keep\n", "B should see A's deletion");

    println!("PASS: two user delete lines");
}

#[test]
fn test_sequential_edits_sync() {
    // Simulate: A edits, saves, B opens, edits, saves, A saves again
    let doc_a = Doc::new();
    let text_a = doc_a.get_or_insert_text("content");
    {
        let mut txn = doc_a.transact_mut();
        text_a.insert(&mut txn, 0, "original\n");
    }

    // A edits
    let old = get_text(&text_a, &doc_a);
    sync_to_yrs(&text_a, &doc_a, &old, "original\nadded by A\n");

    // A pushes to server
    let server_state = encode_state(&doc_a);

    // B opens, pulls server state
    let doc_b = Doc::new();
    let text_b = doc_b.get_or_insert_text("content");
    apply_state(&doc_b, &server_state);
    let b_text = get_text(&text_b, &doc_b);
    assert_eq!(b_text, "original\nadded by A\n");

    // B edits
    sync_to_yrs(&text_b, &doc_b, &b_text, "original\nadded by A\nadded by B\n");

    // B pushes to server
    let server_state_2 = encode_state(&doc_b);

    // A saves again — pulls B's state
    apply_state(&doc_a, &server_state_2);
    let final_a = get_text(&text_a, &doc_a);

    assert_eq!(final_a, "original\nadded by A\nadded by B\n", "A should see B's addition");
    println!("PASS: sequential edits sync");
}

#[test]
fn test_reflow_consistency() {
    // Simulate: yrs initialized with reflowed content,
    // textarea produces same content, diffs should be clean
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");

    // Simulated reflowed content (what textarea would have)
    let reflowed = "This is a long line that was\nwrapped by reflow\n";
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, reflowed);
    }

    let last_synced = reflowed.to_string();

    // textarea_content returns same thing — no diff expected
    let textarea_output = reflowed.to_string();
    sync_to_yrs(&text, &doc, &last_synced, &textarea_output);
    let result = get_text(&text, &doc);
    assert_eq!(result, reflowed, "No-op sync should not change content");

    // Now user adds a word
    let edited = "This is a long line that was\nwrapped by reflow here\n";
    sync_to_yrs(&text, &doc, &textarea_output, edited);
    let result = get_text(&text, &doc);
    assert_eq!(result, edited, "Edit after reflow should work");

    // User deletes a line
    let deleted = "wrapped by reflow here\n";
    sync_to_yrs(&text, &doc, &edited, deleted);
    let result = get_text(&text, &doc);
    assert_eq!(result, deleted, "Delete after reflow should work");

    println!("PASS: reflow consistency");
}

#[test]
fn test_trailing_newline_consistency() {
    // textarea_content always adds trailing \n
    // Make sure yrs stays consistent
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello\n");
    }

    let last = "hello\n".to_string();

    // Simulate: user types, textarea_content returns with trailing \n
    let new1 = "hello\nworld\n";
    sync_to_yrs(&text, &doc, &last, new1);
    assert_eq!(get_text(&text, &doc), "hello\nworld\n");

    // Delete all content — textarea with single empty line returns "\n"
    sync_to_yrs(&text, &doc, new1, "\n");
    assert_eq!(get_text(&text, &doc), "\n");

    // Type again
    sync_to_yrs(&text, &doc, "\n", "new content\n");
    assert_eq!(get_text(&text, &doc), "new content\n");

    println!("PASS: trailing newline consistency");
}

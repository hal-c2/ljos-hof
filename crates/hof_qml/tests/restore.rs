//! Remote panes kept when Hof quits, and picked up when it starts again.
//! A binary of its own: keeping is the process's, and would keep the
//! panes of tests running beside it.

mod support;

use hof_qml::remote;
use support::{host, shown};

#[test]
fn a_restored_pane_picks_up_its_program_where_it_was() {
    let _agent = host("restored");
    let mut first = shown("restored", "echo ready; read x; echo later:$x; read y", -1);
    first.until_text("ready");
    let pane = first.remote_pane();
    // As Hof quitting with a session to restore leaves it.
    remote::keep_programs(true);
    drop(first);
    remote::keep_programs(false);

    let mut again = shown("restored", "echo never", pane);
    again.until_text("ready");
    assert!(!again.text().contains("never"), "{:?}", again.text());
    again.type_text("x\r");
    again.until_text("later:x");
}

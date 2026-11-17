//! What still holds a pane the agent was told to close, for a test's
//! failure to say: a test that only says "the agent still has the pane"
//! leaves whoever reads it to guess whether the program ignored its hangup,
//! another process kept its terminal open, or the agent lost track of it.

/// This process's children and grandchildren as they are now, with what
/// each ignores and blocks and which terminals it has open, and this
/// process's own threads (a pane's reader is `hof-pty-read`).
#[cfg(target_os = "linux")]
pub fn held() -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    use std::fs;

    let read = |path: String| fs::read_to_string(path).unwrap_or_default();
    // pid -> (parent, state, name), from /proc/<pid>/stat: the name is in
    // parentheses and may hold spaces, so fields are counted after the last.
    let mut procs = BTreeMap::new();
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let stat = read(format!("/proc/{pid}/stat"));
        let Some((head, tail)) = stat.rsplit_once(')') else {
            continue;
        };
        let name = head.split_once('(').map_or("", |(_, n)| n).to_owned();
        let mut fields = tail.split_whitespace();
        let state = fields.next().unwrap_or("?").to_owned();
        let parent = fields
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);
        procs.insert(pid, (parent, state, name));
    }
    let me = std::process::id();
    let mut out = String::new();
    let mut ours = vec![me];
    let mut i = 0;
    while let Some(&parent) = ours.get(i) {
        ours.extend(
            procs
                .iter()
                .filter(|(_, p)| p.0 == parent)
                .map(|(&pid, _)| pid),
        );
        i += 1;
    }
    let _ = writeln!(out, "processes under this test ({me}):");
    for &pid in &ours[1..] {
        let (parent, state, name) = &procs[&pid];
        let status = read(format!("/proc/{pid}/status"));
        let mask = |key: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(key))
                .map_or(String::new(), |v| v.trim().to_owned())
        };
        let ttys: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|fd| fs::read_link(fd.path()).ok())
            .map(|t| t.display().to_string())
            .filter(|t| t.starts_with("/dev/pts/") || t == "/dev/ptmx")
            .collect();
        let command = read(format!("/proc/{pid}/cmdline")).replace('\0', " ");
        let _ = writeln!(
            out,
            "  {pid} (parent {parent}) {state} {name}: {} | ignores {} blocks {} | terminals {ttys:?}",
            command.trim(),
            mask("SigIgn:"),
            mask("SigBlk:"),
        );
    }
    if ours.len() == 1 {
        let _ = writeln!(out, "  none: every program the test started has ended");
    }
    // The terminals this process still holds the master side of, and any
    // process anywhere that has their other side open: one outside this
    // test's tree keeps a terminal open as well as one inside it.
    let held: Vec<String> = fs::read_dir("/proc/self/fdinfo")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|fd| {
            read(fd.path().display().to_string()).lines().find_map(|l| {
                l.strip_prefix("tty-index:")
                    .map(|n| format!("/dev/pts/{}", n.trim()))
            })
        })
        .collect();
    let _ = writeln!(
        out,
        "terminals this process holds the master side of: {held:?}"
    );
    for (&pid, (_, _, name)) in &procs {
        let open: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|fd| fs::read_link(fd.path()).ok())
            .map(|t| t.display().to_string())
            .filter(|t| held.contains(t))
            .collect();
        if !open.is_empty() {
            let command = read(format!("/proc/{pid}/cmdline")).replace('\0', " ");
            let _ = writeln!(out, "  open in {pid} {name} ({}): {open:?}", command.trim());
        }
    }
    let mut threads: BTreeMap<String, usize> = BTreeMap::new();
    for task in fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
    {
        let comm = read(format!("{}/comm", task.path().display()));
        *threads.entry(comm.trim().to_owned()).or_default() += 1;
    }
    let _ = write!(out, "this process's threads: {threads:?}");
    out
}

#[cfg(not(target_os = "linux"))]
pub fn held() -> String {
    String::from("(what holds it is only looked up on Linux)")
}

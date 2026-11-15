{
 let art = std::path::Path::new("/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix");
 let build = art.join("build-4d32f19d4");
 let extension = build.join("spur_graph.duckdb_extension");
 let python = "/private/tmp/spur-graph-duckdb-v2-20261114/bin/python";
 let mut runs = Vec::new();
 for (name, script, tail) in [
  ("validation-from-notebook", "validate_extension.py", vec!["--mode", "fixed"]),
  ("scope-probe-from-notebook", "probe_scope.py", vec![]),
  ("path-probe-from-notebook", "probe_paths.py", vec!["--expect-fixed"]),
 ] {
  let output = build.join(format!("{name}.json"));
  let mut args=vec![art.join(script).to_str().unwrap().to_owned(),"--extension".to_owned(),extension.to_str().unwrap().to_owned(),"--output".to_owned(),output.to_str().unwrap().to_owned()];
  args.extend(tail.into_iter().map(str::to_owned));
  let result=std::process::Command::new(python).args(&args).output()?;
  std::fs::write(build.join(format!("{name}.stdout.txt")),&result.stdout)?;
  std::fs::write(build.join(format!("{name}.stderr.txt")),&result.stderr)?;
  runs.push(serde_json::json!({"name":name,"python":python,"argv":args,"exit_code":result.status.code()}));
  std::fs::write(build.join("notebook-commands.json"),serde_json::to_vec_pretty(&runs)?)?;
  assert!(result.status.success(),"Fresh DuckDB2 validation process failed; inspect saved logs");
  let report:serde_json::Value=serde_json::from_slice(&std::fs::read(&output)?)?;
  println!("{}: DuckDB {}, extension SHA256 {}",name,report["duckdb_version"],report["extension_sha256"]);
  if name=="scope-probe-from-notebook" {
   assert_eq!(report["all_positive_controls_passed"],true);
   println!("Six single-qualifier controls: {}",report["expected_local_constructor_preserved"]);
  } else {
   assert_eq!(report["all_checks_passed"],true);
   if name=="path-probe-from-notebook" {
    println!("Seven multi-segment positives plus qualified standard-library negative: {}",report["cases"]);
   } else {
    for receipt in report["receipts"].as_array().unwrap() {
     if ["selection","selected_call_counts","real_constructor_control"].contains(&receipt["name"].as_str().unwrap_or("")) {
      println!("{}: {}",receipt["name"],receipt["rows"]);
     }
    }
   }
  }
 }
}

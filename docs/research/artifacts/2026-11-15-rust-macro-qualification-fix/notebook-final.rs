{
 let art = std::path::Path::new("/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-rust-macro-qualification-fix");
 let final_dir = art.join("final-99760f547");
 let extension = final_dir.join("spur_graph.duckdb_extension");
 let python = "/private/tmp/spur-graph-duckdb-v2-20261114/bin/python";
 let mut runs = Vec::new();
 for (name, script, tail) in [
  ("validation-from-notebook", "validate_extension.py", vec!["--mode", "fixed"]),
  ("scope-probe-from-notebook", "probe_scope.py", vec![]),
 ] {
  let output_path = final_dir.join(format!("{name}.json"));
  let mut args = vec![art.join(script).to_str().unwrap().to_owned(), "--extension".to_owned(), extension.to_str().unwrap().to_owned(), "--output".to_owned(), output_path.to_str().unwrap().to_owned()];
  args.extend(tail.into_iter().map(str::to_owned));
  let result = std::process::Command::new(python).args(&args).output()?;
  std::fs::write(final_dir.join(format!("{name}.stdout.txt")), &result.stdout)?;
  std::fs::write(final_dir.join(format!("{name}.stderr.txt")), &result.stderr)?;
  runs.push(serde_json::json!({"name":name,"python":python,"argv":args,"exit_code":result.status.code()}));
  std::fs::write(final_dir.join("notebook-commands.json"),serde_json::to_vec_pretty(&runs)?)?;
  assert!(result.status.success(), "Fresh DuckDB2 process failed; inspect saved stdout/stderr");
  let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&output_path)?)?;
  println!("{}: DuckDB {}, extension SHA256 {}",name,report["duckdb_version"],report["extension_sha256"]);
  if name == "validation-from-notebook" {
   assert_eq!(report["all_checks_passed"],true);
   for receipt in report["receipts"].as_array().unwrap() {
    if ["selection","selected_call_counts","real_constructor_control"].contains(&receipt["name"].as_str().unwrap_or("")) {
     println!("{}: {}",receipt["name"],receipt["rows"]);
    }
   }
  } else {
   assert_eq!(report["all_positive_controls_passed"],true);
   println!("Six scope controls: {}",report["expected_local_constructor_preserved"]);
   println!("Grounded target ownership: {}",report["rows"]);
  }
 }
}

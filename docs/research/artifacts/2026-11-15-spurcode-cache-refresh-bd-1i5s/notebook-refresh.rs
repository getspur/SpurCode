{
 let art=std::path::Path::new("/Users/kevintruong/project/SpurCode/docs/research/artifacts/2026-11-15-spurcode-cache-refresh-bd-1i5s");
 let python="/private/tmp/spur-graph-duckdb-v2-20261114/bin/python";
 let mut runs=Vec::new();
 for phase in ["ingest","replay","verify","publish"] {
  let args=vec![art.join("refresh.py").to_str().unwrap().to_owned(),"--phase".to_owned(),phase.to_owned()];
  let result=std::process::Command::new(python).args(&args).output()?;
  std::fs::write(art.join(format!("{phase}.stdout.txt")),&result.stdout)?;
  std::fs::write(art.join(format!("{phase}.stderr.txt")),&result.stderr)?;
  runs.push(serde_json::json!({"phase":phase,"python":python,"argv":args,"exit_code":result.status.code()}));
  std::fs::write(art.join("notebook-commands.json"),serde_json::to_vec_pretty(&runs)?)?;
  println!("{}: {}",phase,String::from_utf8_lossy(&result.stdout));
  assert!(result.status.success(),"Fresh cache refresh phase failed; inspect saved stderr and progress.json");
 }
 let report:serde_json::Value=serde_json::from_slice(&std::fs::read(art.join("phase-verify.json"))?)?;
 assert_eq!(report["all_checks_passed"],true);
 for receipt in report["receipts"].as_array().unwrap() {
  if ["rust_calls_by_version","real_vec_control","cache_versions"].contains(&receipt["name"].as_str().unwrap_or("")) {
   println!("{}: {}",receipt["name"],receipt["rows"]);
  }
 }
 println!("Published cache: {}",String::from_utf8_lossy(&std::fs::read(art.join("published.json"))?));
}

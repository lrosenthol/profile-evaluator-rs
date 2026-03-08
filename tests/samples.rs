/*
Copyright 2026 Adobe. All rights reserved.
This file is licensed to you under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License. You may obtain a copy
of the License at http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software distributed under
the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR REPRESENTATIONS
OF ANY KIND, either express or implied. See the License for the specific language
governing permissions and limitations under the License.
*/

use std::fs;
use std::path::Path;

use profile_evaluator_rs::{OutputFormat, evaluate_files, serialize_report};
use serde_json::Value;

fn run_case(name: &str) {
    let profile = format!("testfiles/{}_profile.yml", name);
    let indicators = format!("testfiles/{}_indicators.json", name);

    let report = evaluate_files(&profile, &indicators).expect("evaluation should succeed");

    let expected_json_path = format!("output/{}_indicators_report.json", name);
    let expected_json: Value =
        serde_json::from_str(&fs::read_to_string(&expected_json_path).expect("expected json file"))
            .expect("valid expected json");
    assert_eq!(report, expected_json, "json mismatch for case {name}");

    let actual_yaml = serialize_report(&report, OutputFormat::Yaml).expect("yaml serialization");
    let actual_yaml_value: Value = serde_yaml::from_str(&actual_yaml).expect("actual yaml parse");

    let expected_yaml_path = format!("output/{}_indicators_report.yml", name);
    let expected_yaml_text = fs::read_to_string(&expected_yaml_path).expect("expected yaml file");
    let expected_yaml_value: Value = serde_yaml::from_str(&expected_yaml_text).expect("expected yaml parse");

    assert_eq!(
        actual_yaml_value, expected_yaml_value,
        "yaml mismatch for case {name}"
    );
}

#[test]
fn validates_all_sample_profiles() {
    let cases = [
        "blocks",
        "camera",
        "expression",
        "genai",
        "includes",
        "no_manifests",
        "signature",
    ];

    for case in cases {
        run_case(case);
    }
}

#[test]
fn cli_roundtrip_json_matches_expected_case() {
    let report = evaluate_files("testfiles/no_manifests_profile.yml", "testfiles/no_manifests_indicators.json")
        .expect("evaluation should succeed");
    let rendered = serialize_report(&report, OutputFormat::Json).expect("json serialization");
    let rendered_value: Value = serde_json::from_str(&rendered).expect("valid json");

    let expected_path = Path::new("output/no_manifests_indicators_report.json");
    let expected: Value = serde_json::from_str(&fs::read_to_string(expected_path).expect("read expected"))
        .expect("parse expected json");

    assert_eq!(rendered_value, expected);
}

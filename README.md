# profile-evaluator-rs

Evaluate an asset profile (YAML) against indicators (JSON). Produces a report by running profile expressions and template text over the indicator data.

## Building

Requires Rust (e.g. via [rustup](https://rustup.rs/)).

This crate depends on a local `json-formula-rs` at `../json-formula-rs`. Clone or symlink that repo as a sibling of this one, then:

```bash
cargo build --release
```

## Usage

```bash
profile-evaluator --profile <PROFILE> --indicators <INDICATORS> [OPTIONS]
```

| Option | Short | Description |
|--------|--------|-------------|
| `--profile` | `-p` | Path to the asset profile YAML file |
| `--indicators` | `-i` | Path to the indicators JSON file |
| `--format` | `-f` | Output format: `json` (default) or `yaml` |
| `--output` | `-o` | Write report to this file (default: stdout) |

### Examples

```bash
# JSON report to stdout
profile-evaluator -p profile.yml -i indicators.json

# YAML report to a file
profile-evaluator -p profile.yml -i indicators.json -f yaml -o report.yml
```

## Development

Run tests (expects `testfiles/` and `output/` with sample profiles and expected reports):

```bash
cargo test
```

## License

See repository for license information.

# Publish Workflow Specification

## GitHub Actions: `.github/workflows/publish.yml`

### Trigger

Manual dispatch with version bump selection:

```yaml
on:
  workflow_dispatch:
    inputs:
      bump:
        description: 'Version bump type'
        required: true
        type: choice
        options:
          - patch
          - minor
          - major
        default: 'patch'
```

### Workflow Steps

#### 1. Version Bump

1. Read current version from `Cargo.toml` (`[package] version = "x.y.z"`).
2. Based on `inputs.bump`:
   - `patch`: `x.y.z` → `x.y.(z+1)`
   - `minor`: `x.y.z` → `x.(y+1).0`
   - `major`: `x.y.z` → `(x+1).0.0`
3. Update `Cargo.toml` with the new version.
4. Run `cargo generate-lockfile` to update `Cargo.lock`.
5. Commit: `release: vX.Y.Z`
6. Create git tag: `vX.Y.Z`
7. Push commit + tag to `main`.

#### 2. Publish to crates.io

```yaml
- run: cargo publish
  env:
    CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

#### 3. Docker Image

```yaml
- uses: docker/setup-qemu-action@v3
- uses: docker/setup-buildx-action@v3
- uses: docker/login-action@v3
  with:
    username: ${{ secrets.DOCKERHUB_USERNAME }}
    password: ${{ secrets.DOCKERHUB_TOKEN }}
- uses: docker/build-push-action@v5
  with:
    push: true
    platforms: linux/amd64,linux/arm64
    tags: |
      r12f/otel-modbus-exporter:vX.Y.Z
      r12f/otel-modbus-exporter:latest
```

#### 4. GitHub Release

Create a GitHub Release for the tag `vX.Y.Z`:

```yaml
- uses: softprops/action-gh-release@v2
  with:
    tag_name: vX.Y.Z
    name: vX.Y.Z
    generate_release_notes: true
```

This auto-generates release notes from merged PRs since the last tag.

### Job Order

All steps run in a single job (sequential):
1. Checkout → version bump → commit + tag + push
2. `cargo publish` (crates.io)
3. Docker build + push (multi-arch)
4. GitHub Release creation

### Required Secrets

| Secret | Purpose |
|--------|---------|
| `CARGO_REGISTRY_TOKEN` | crates.io publish token |
| `DOCKERHUB_USERNAME` | Docker Hub username |
| `DOCKERHUB_TOKEN` | Docker Hub access token |

### Required Permissions

```yaml
permissions:
  contents: write   # push commits/tags + create releases
```

### Notes

- The workflow needs a PAT or `contents: write` permission to push the version bump commit and tag.
- No `v*` tag trigger — fully manual via `workflow_dispatch` to prevent accidental publishes.
- The version in `Cargo.toml` is the source of truth.

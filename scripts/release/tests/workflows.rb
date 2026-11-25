require 'yaml'
require 'json'
require 'minitest/autorun'
require 'tmpdir'
require 'open3'

class ReleaseWorkflows < Minitest::Test
  DIR = File.expand_path('../../../.github/workflows', __dir__)
  BUILD_RUNNERS = {
    'x86_64-unknown-linux-gnu' => 'ubuntu-latest',
    'aarch64-unknown-linux-gnu' => 'ubuntu-latest',
    'x86_64-unknown-linux-musl' => 'ubuntu-latest',
    'x86_64-apple-darwin' => 'macos-latest',
    'aarch64-apple-darwin' => 'macos-latest',
    'x86_64-pc-windows-msvc' => 'windows-latest'
  }.freeze
  def workflow(name)
    YAML.safe_load(File.read(File.join(DIR, name + '.yml')))
  end
  def commands(job)
    job.fetch('steps').map { |step| step['run'] }.compact.join("\n")
  end
  def edges(jobs, name)
    Array(jobs.fetch(name)['needs'])
  end
  def ancestors(jobs, name)
    edges(jobs, name).flat_map { |dep| [dep] + ancestors(jobs, dep) }.uniq
  end
  def test_workflows_separate_triggers_and_classify_stripped_tags
    %w[release release-crapts].each do |name|
      w = workflow(name)
      prefix = name == 'release' ? 'v' : 'crapts-v'
      assert_equal({'push' => {'tags' => [prefix + '[0-9]+.[0-9]+.[0-9]+*']}}, w.fetch(true))
      jobs = w.fetch('jobs')
      assert_includes commands(jobs['preflight']), 'check-tag.sh ' + (name == 'release' ? 'cargo-crap' : 'crapts')
      assert_includes commands(jobs['preflight']), 'check-core.sh'
      assert_equal({'contents'=>'read'},jobs['preflight']['permissions'])
      assert_includes edges(jobs, 'build'), 'preflight'
      assert_includes edges(jobs, 'release'), 'build'
      assert_equal({'contents'=>'write'},jobs['release']['permissions'])
      product = name == 'release' ? 'cargo-crap' : 'crapts'
      assert_equal './.github/workflows/_build.yml', jobs['build']['uses']
      inputs = jobs['build']['with']
      assert_equal [product, product, 'triage'], inputs.values_at('package','binary','features')
      targets = JSON.parse(inputs.fetch('targets'))
      expected = %w[aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc]
      expected << 'x86_64-unknown-linux-musl' if name == 'release-crapts'
      assert_equal expected.sort, targets.map { |v| v['target'] }.sort
      targets.each do |row|
        assert_equal BUILD_RUNNERS.fetch(row.fetch('target')), row.fetch('os')
        assert_equal %w[aarch64-unknown-linux-gnu x86_64-unknown-linux-musl].include?(row['target']), row['use_cross']
      end
      assert_includes commands(jobs['release']), 'SHA256SUMS'
      release = jobs['release']['steps'].find { |s| s['uses'].to_s.include?('action-gh-release') }.fetch('with')
      if name == 'release'
        refute_equal 'false',release['make_latest'].to_s
        assert_equal ['release'],edges(jobs, 'action-smoke')
        assert_equal ['action-smoke'],edges(jobs, 'move-v1')
        refute_includes ancestors(jobs,'move-v1'), 'publish'
      else
        assert_equal 'false',release['make_latest'].to_s
        assert_includes release['prerelease'],'needs.preflight.outputs.is_prerelease'
        assert_equal %w[is_prerelease major version], jobs['preflight']['outputs'].keys.sort
      end
    end
  end
  def test_prerequisite_failures_block_every_dependent_publication_and_promotion
    jobs = workflow('release-crapts').fetch('jobs')
    stages = %w[preflight build release archive-smoke publish-crates publish-npm npm-smoke promote-npm]
    stages.each_with_index do |name,i|
      assert_empty stages.take(i) - ancestors(jobs, name)
      assert_includes edges(jobs,name), stages[i-1] if i > 0
      refute_match(/always\(|failure\(|continue-on-error/, jobs.fetch(name).inspect)
    end
    assert_match(/needs\.preflight\.outputs\.is_prerelease == 'false'/,jobs['promote-npm']['if'])
    assert_equal %w[contents id-token], jobs['promote-npm']['permissions'].keys.sort
    %w[publish-npm promote-npm].each do |name|
      assert_equal({'contents'=>'read','id-token'=>'write'}, jobs[name]['permissions'])
      refute_match(/NODE_AUTH_TOKEN|NPM_TOKEN/, jobs[name].inspect)
      assert_includes commands(jobs[name]),'npm@11.21.0'
    end
    other = workflow('release')['jobs']['publish']
    [other,jobs['publish-crates']].each do |job|
      checkout = job.fetch('steps').find { |step| step['uses'].to_s.start_with?('actions/checkout@') }
      assert_equal [0, true], checkout.fetch('with').values_at('fetch-depth', 'fetch-tags')
      assert_equal({'contents'=>'write'},job['permissions'])
      assert_includes job['env']['CARGO_REGISTRY_TOKEN'],'secrets.CARGO_REGISTRY_TOKEN'
      run = commands(job)
      assert_operator run.index('publish-core.sh'), :<, run.index('publish-frontend.sh')
    end
    assert_equal other['concurrency'], jobs['publish-crates']['concurrency']
    targets = JSON.parse(jobs['build']['with']['targets']).map { |row| row['target'] }
    smoke = jobs['archive-smoke']['strategy']['matrix']['include']
    assert_equal targets.sort, smoke.map { |row| row['target'] }.sort
    assert_equal BUILD_RUNNERS.merge('aarch64-unknown-linux-gnu' => 'ubuntu-24.04-arm'),
                 smoke.to_h { |row| row.values_at('target', 'os') }
    assert_equal '${{ matrix.os }}', jobs['archive-smoke']['runs-on']
    rosetta = jobs['archive-smoke']['steps'].find { |s| s['run'].to_s.include?('softwareupdate --install-rosetta') }
    assert_equal "matrix.target == 'x86_64-apple-darwin'", rosetta.fetch('if')
    assert_includes commands(jobs['archive-smoke']), 'verify-archives.mjs'
    assert_includes commands(jobs['archive-smoke']), 'smoke.mjs'
    assert_includes commands(jobs['publish-npm']), 'verify-archives.mjs'
    assert_includes commands(jobs['publish-npm']), 'stage-npm.mjs'
    assert_includes commands(jobs['publish-npm']), 'verify-archives.mjs artifacts --staged staged'
    assert_includes commands(jobs['publish-npm']), 'publish-npm.mjs'
    assert_equal %w[alpine macos ubuntu windows], jobs['npm-smoke']['strategy']['matrix']['include'].map { |row| row['host'] }.sort
    alpine = jobs['npm-smoke']['steps'].find { |s| s['if'] == "matrix.host == 'alpine'" }
    assert_includes alpine.fetch('run'), 'node:24-alpine'
    assert_includes alpine.fetch('run'), '--expect-package @minikin/crapts-linux-x64-musl'
    assert_includes commands(jobs['promote-npm']), 'npm dist-tag add'
  end
  def test_tagged_action_smokes_gate_both_promotions
    jobs = workflow('release-crapts').fetch('jobs')
    smoke = jobs.fetch('crapts-action-smoke')
    assert_equal %w[npm-smoke preflight], edges(jobs, 'crapts-action-smoke').sort
    assert_nil smoke['if']
    assert_equal({'contents' => 'read'}, smoke.fetch('permissions'))
    assert_equal %w[macos-latest ubuntu-latest windows-latest], smoke.dig('strategy', 'matrix', 'os').sort
    assert_equal '${{ matrix.os }}', smoke.fetch('runs-on')
    assert_equal '${{ needs.preflight.outputs.version }}', smoke.dig('env', 'VERSION')
    setup = smoke.fetch('steps').index { |s| s['uses'].to_s.start_with?('actions/setup-node@') && s.dig('with', 'node-version').to_s == '24' }
    fixture = smoke.fetch('steps').index { |s| s['id'] == 'fixture' }
    assert_operator setup, :<, fixture
    checkout = smoke.fetch('steps').find { |s| s['uses'].to_s.start_with?('actions/checkout@') }
    assert_equal '${{ github.sha }}', checkout.dig('with', 'ref')
    actions = smoke.fetch('steps').select { |s| s['uses'] == './actions/crapts' }
    assert_equal %w[push-score pr-score], actions.map { |s| s['id'] }
    actions.each { |s| assert_empty s.fetch('with').keys & %w[version package] }
    assert_equal ['pr-score'], smoke.fetch('steps').select { |s| s['continue-on-error'] }.map { |s| s['id'] }
    assert_match(/smoke.py pr/, smoke.fetch('steps').last.fetch('run'))
    %w[promote-npm move-crapts-v1].each do |name|
      assert_equal %w[crapts-action-smoke npm-smoke preflight], edges(jobs, name).sort
      assert_includes jobs[name].fetch('if'), "needs.preflight.outputs.is_prerelease == 'false'"
      refute_match(/always\(|failure\(|continue-on-error/, jobs[name].inspect)
    end
    refute_includes jobs['promote-npm']['if'], 'major'
    tag = jobs.fetch('move-crapts-v1')
    assert_equal({'contents' => 'write'}, tag.fetch('permissions'))
    %w[0 1].each { |major| assert_includes tag.fetch('if'), "needs.preflight.outputs.major == '#{major}'" }
    assert_includes commands(tag), 'move-tag.sh'
  end
  def test_frontend_publication_skips_existing_versions_and_fails_closed_on_registry_errors
    script = File.expand_path('../publish-frontend.sh', __dir__)
    Dir.mktmpdir do |dir|
      cargo = File.join(dir, 'cargo')
      curl = File.join(dir, 'curl')
      File.write(cargo, "#!/bin/sh\nprintf '%s' \"$*\" > \"$UPLOAD\"\n")
      File.write(curl, "#!/bin/sh\nprintf '%s' \"$STUB_STATUS\"\n")
      File.chmod(0755, cargo, curl)
      upload = File.join(dir, 'upload')
      env = {'PATH'=>dir + ':' + ENV.fetch('PATH'), 'UPLOAD'=>upload}
      ['200', '503', '404'].each do |status|
        out, result = Open3.capture2e(env.merge('STUB_STATUS'=>status),'bash',script,'crapts','0.1.0')
        assert_equal(status != '503',result.success?,out)
        assert_equal(status == '404',File.exist?(upload),out)
      end
      assert_equal 'publish --locked -p crapts', File.read(upload)
    end
  end
  def test_frontend_publication_retries_only_core_index_lag_and_stops_after_three_attempts
    script = File.expand_path('../publish-frontend.sh', __dir__)
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, 'curl'), "#!/bin/sh\necho 404\n")
      File.write(File.join(dir, 'sleep'), "#!/bin/sh\necho \"$1\" >> waits\n")
      File.write(File.join(dir, 'cargo'), <<~'SH')
        #!/bin/sh
        echo "$*" >> attempts
        if [ "$(wc -l < attempts)" -le "$FAILURES" ]; then
          echo "$DIAGNOSTIC" >&2
          exit 101
        fi
      SH
      File.chmod(0755, *%w[cargo curl sleep].map { |name| File.join(dir, name) })
      missing = 'error: no matching package named `cargo-crap-core` found'
      mismatch = 'error: failed to select a version for the requirement `cargo-crap-core = "=0.2.0"`'
      denied = 'error: permission denied while publishing a crate that depends on cargo-crap-core'
      [[missing, 1, 2, 0], [mismatch, 3, 3, 101], [denied, 1, 1, 101]].each do |message, failures, attempts, code|
        File.write(File.join(dir, 'attempts'), '')
        File.write(File.join(dir, 'waits'), '')
        env = {'PATH'=>dir + ':' + ENV.fetch('PATH'), 'FAILURES'=>failures.to_s, 'DIAGNOSTIC'=>message}
        out, status = Open3.capture2e(env, 'bash', script, 'crapts', '0.2.0', chdir: dir)
        assert_equal code, status.exitstatus, out
        assert_includes out, message
        assert_equal ['publish --locked -p crapts'] * attempts, File.readlines(File.join(dir, 'attempts'), chomp: true)
        assert_equal attempts - 1, File.readlines(File.join(dir, 'waits')).size
      end
    end
  end
  def test_pull_requests_link_and_inspect_both_linux_release_archives_before_tagging
    jobs = workflow('ci').fetch('jobs')
    build = jobs.fetch('crapts-cross')
    assert_equal './.github/workflows/_build.yml', build.fetch('uses')
    assert_equal ['changes'], edges(jobs, 'crapts-cross')
    assert_includes build.fetch('if'), "github.event_name == 'pull_request'"
    assert_includes build.fetch('if'), "needs.changes.outputs.rust == 'true'"
    inputs = build.fetch('with')
    assert_equal %w[crapts crapts triage], inputs.values_at('package', 'binary', 'features')
    release = workflow('release-crapts').dig('jobs', 'build', 'with')
    expected = JSON.parse(release.fetch('targets')).select { |row| row.fetch('use_cross') }
    assert_equal %w[aarch64-unknown-linux-gnu x86_64-unknown-linux-musl], expected.map { |row| row.fetch('target') }.sort
    assert_equal expected.sort_by { |row| row.fetch('target') }, JSON.parse(inputs.fetch('targets')).sort_by { |row| row.fetch('target') }
    filters = jobs.dig('changes', 'steps').find { |s| s['id'] == 'filter' }.dig('with', 'filters')
    rust = YAML.safe_load(filters).fetch('rust')
    %w[crates/crapts/** crates/cargo-crap-core/** Cargo.toml **/Cargo.toml Cargo.lock .github/workflows/_build.yml .github/workflows/release-crapts.yml].each { |pattern| assert_includes rust, pattern }
    inspection = workflow('_build').dig('jobs', 'build', 'steps').find { |s| s['name'] == 'Inspect Linux cross archive' }
    assert_includes inspection.fetch('if'), 'matrix.use_cross'
    %w[tar readelf AArch64 X86-64].each { |token| assert_includes inspection.fetch('run'), token }
    inspect_script = inspection.fetch('run')
    assert_includes inspect_script, 'if [[ "$TARGET" == x86_64-unknown-linux-musl ]]'
    assert_includes inspect_script, 'readelf -d "$unpacked/$BINARY"'
    assert_match(/if grep -q NEEDED .*?exit 1/m, inspect_script)
    assert_includes commands(jobs.fetch('npm-wrapper')), 'cargo build --locked -p crapts --features triage'
  end
  def test_reusable_build_and_ci_contract
    w = workflow('_build')
    assert_equal %w[binary features package targets],w.fetch(true).fetch('workflow_call')['inputs'].keys.sort
    build = w.fetch('jobs').fetch('build')
    assert_equal '${{ matrix.os }}', build.fetch('runs-on')
    install = build.fetch('steps').find { |s| s.dig('with', 'tool') == 'cross' }
    assert_equal 'matrix.use_cross', install.fetch('if')
    compile = build.fetch('steps').find { |s| s['name'] == 'Build' }
    assert_equal "${{ matrix.use_cross && 'cross' || 'cargo' }}", compile.fetch('env').fetch('BUILDER')
    assert_includes compile.fetch('run'), '"$BUILDER" build "${args[@]}"'
    run = commands(build)
    %w[--locked --release --features --target .tar.gz .zip].each { |token| assert_includes run, token }
    ci = workflow('ci')['jobs']
    assert_equal %w[macos-latest ubuntu-latest windows-latest], ci['npm-wrapper']['strategy']['matrix']['os'].sort
    assert_includes ci['npm-wrapper']['if'], 'needs.changes.outputs.npm'
    assert_includes commands(ci['npm-wrapper']), 'CRAPTS_BINARY'
    assert_includes commands(ci['npm-wrapper']), 'node --test'
    [ci['release-scripts'], workflow('action')['jobs']['scripts']].each do |job|
      line = commands(job).lines.find { |l| l.include?('./actionlint -ignore SC2016') }
      %w[_build release release-crapts ci action demo].each { |file| assert_includes line, ".github/workflows/#{file}.yml" }
    end
  end
end

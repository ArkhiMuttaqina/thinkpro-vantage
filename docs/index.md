---
layout: page
title: ThinkPro Vantage
description: Native ThinkPad control suite for Linux — fan curves, battery thresholds, CPU and GPU tuning, and a safe MCP server, in one fast Tauri app.
---

<div class="landing">

  <!-- ============ HERO ============ -->
  <section class="hero">
    <div class="hero-bg">
      <span class="blob b1"></span><span class="blob b2"></span><span class="blob b3"></span>
      <div class="grid-overlay"></div>
    </div>
    <div class="hero-inner">
      <span class="hero-badge">Linux-only · Open Source · LGPL-3.0</span>
      <h1 class="hero-title">ThinkPro <span class="grad">Vantage</span></h1>
      <p class="hero-tag">Unlock every knob on your ThinkPad — fan curves, dual-battery charge thresholds, CPU governor &amp; P-state, GPU clocks, and a safe localhost MCP server — from one native, zero-bloat app.</p>
      <div class="hero-actions">
        <a class="btn btn-brand" href="./download">Get Started</a>
        <a class="btn btn-ghost" href="https://github.com/ArkhiMuttaqina/thinkpro-vantage">GitHub</a>
        <a class="btn btn-ghost" href="https://github.com/ArkhiMuttaqina/thinkpro-vantage/stargazers">★ Star</a>
      </div>
      <div class="hero-terminal">
        <div class="term-bar"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span><span class="term-title">install — one command</span></div>
        <div class="term-body"><span class="prompt">$</span>sudo apt install ./thinkpro-vantage_*.deb<span class="cursor"></span></div>
      </div>
    </div>
  </section>

  <!-- ============ TICKER ============ -->
  <div class="ticker" aria-hidden="true">
    <div class="ticker-track">
      <span class="ticker-item">Fan Curves</span><span class="ticker-item">Battery Thresholds</span><span class="ticker-item">CPU Governor</span><span class="ticker-item">P-State / EPP</span><span class="ticker-item">GPU Clock (LACT)</span><span class="ticker-item">Power Daemon (TLP/PPD)</span><span class="ticker-item">PAM Guard</span><span class="ticker-item">MCP Server</span><span class="ticker-item">ClamAV Scan</span>
      <span class="ticker-item">Fan Curves</span><span class="ticker-item">Battery Thresholds</span><span class="ticker-item">CPU Governor</span><span class="ticker-item">P-State / EPP</span><span class="ticker-item">GPU Clock (LACT)</span><span class="ticker-item">Power Daemon (TLP/PPD)</span><span class="ticker-item">PAM Guard</span><span class="ticker-item">MCP Server</span><span class="ticker-item">ClamAV Scan</span>
    </div>
  </div>

  <!-- ============ STATS ============ -->
  <section class="stats">
    <div class="wrap stats-grid">
      <div class="stat reveal"><b>9<i>+</i></b><span>hardware controls</span></div>
      <div class="stat reveal" style="--d:80ms"><b>2</b><span>architectures — amd64 &amp; arm64</span></div>
      <div class="stat reveal" style="--d:160ms"><b>147<i>+</i></b><span>CI-gated tests</span></div>
      <div class="stat reveal" style="--d:240ms"><b>0</b><span>telemetry — fully offline</span></div>
    </div>
  </section>

  <!-- ============ FEATURES ============ -->
  <section>
    <div class="wrap">
      <div class="section-head reveal">
        <span class="section-kicker">What you control</span>
        <h2 class="section-title">Every dial, one window</h2>
        <p class="section-sub">Stop shelling into sysfs. ThinkPro Vantage wires the real ThinkPad controls into a fast, clear UI.</p>
      </div>
      <div class="cards">
        <div class="card reveal"><span class="icon">🌀</span><h3>Fan Control</h3><p>EC fan levels and editable fan curves, driven by live temperature — from silent to max, safely through a polkit-scoped helper.</p></div>
        <div class="card reveal" style="--d:70ms"><span class="icon">🔋</span><h3>Battery Care</h3><p>Dual-battery charge thresholds to keep cells below the 80% wear zone, with per-source start/stop control.</p></div>
        <div class="card reveal" style="--d:140ms"><span class="icon">⚡</span><h3>CPU &amp; P-State</h3><p>Governor switching, boost toggle and clock ceilings on top of AMD P-State EPP or Intel P-State HWP.</p></div>
        <div class="card reveal" style="--d:210ms"><span class="icon">🎛️</span><h3>GPU Clock (LACT)</h3><p>AMD GPU ceiling from the levels the card actually exposes — re-applied automatically when you switch AC/battery.</p></div>
        <div class="card reveal" style="--d:280ms"><span class="icon">🔌</span><h3>Power Daemon</h3><p>TLP vs power-profiles-daemon arbitration, told plainly: who owns each knob, and what will actually stick.</p></div>
        <div class="card reveal" style="--d:350ms"><span class="icon">📊</span><h3>System Monitor</h3><p>Per-core CPU, memory, disk and network — plus a process view, all on a live dashboard.</p></div>
      </div>
    </div>
  </section>

  <!-- ============ ADVANTAGES ============ -->
  <section style="border-top:1px solid var(--tp-border)">
    <div class="wrap">
      <div class="section-head reveal">
        <span class="section-kicker">Why ThinkPro Vantage</span>
        <h2 class="section-title">Built right, not just built</h2>
        <p class="section-sub">The boring engineering choices that make it trustworthy.</p>
      </div>
      <div class="cards">
        <div class="card reveal"><span class="icon">🦀</span><h3>Native Rust + Tauri</h3><p>A ~30 MB binary and a fraction of the RAM of an Electron app — no webview bloat, no background node.</p></div>
        <div class="card reveal" style="--d:70ms"><span class="icon">🔒</span><h3>Loopback-only MCP</h3><p>The MCP server binds 127.0.0.1 and allowlists <code>Host</code> + <code>Origin</code> — no DNS-rebinding, no CSRF against localhost.</p></div>
        <div class="card reveal" style="--d:140ms"><span class="icon">🛡️</span><h3>Minimal root surface</h3><p>Fan control runs through a <code>thinkpro-ctl</code> helper with a polkit rule matched only to it — passwordless, never a root app.</p></div>
        <div class="card reveal" style="--d:210ms"><span class="icon">🧭</span><h3>amd64 + arm64</h3><p>One codebase, two native builds — tested on x86 ThinkPads and arm64 alike, with graceful degradation when a sensor is absent.</p></div>
        <div class="card reveal" style="--d:280ms"><span class="icon">🔭</span><h3>Zero telemetry</h3><p>Everything stays on your machine. Settings sync is your own JSON export — no account, no analytics, no phone-home.</p></div>
        <div class="card reveal" style="--d:350ms"><span class="icon">📜</span><h3>LGPL-3.0 open source</h3><p>Auditable, forkable, and derived transparently from ThinkUtils. Your hardware, your code.</p></div>
      </div>
    </div>
  </section>

  <!-- ============ INSTALL ============ -->
  <section class="install">
    <div class="wrap install-grid">
      <div class="reveal">
        <span class="section-kicker">Install</span>
        <h2 class="section-title">Up and control in a minute</h2>
        <p class="section-sub">Packages for every major Linux family — and a universal AppImage.</p>
        <div class="distro">
          <a href="./download">📦 .deb — Debian / Ubuntu</a>
          <a href="./download">📦 .rpm — Fedora / RHEL</a>
          <a href="./download">🧰 AppImage</a>
          <a href="https://github.com/ArkhiMuttaqina/thinkpro-vantage/tree/main/packaging/aur">🏗️ Arch PKGBUILD</a>
        </div>
      </div>
      <div class="install-term reveal" style="--d:120ms">
        <div class="term-bar"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span><span class="term-title">bash</span></div>
        <div class="term-body">
          <span class="ln cmt"># Debian / Ubuntu (from the apt repo)</span>
          <span class="ln cmd"><span class="prompt">$</span>echo 'deb [trusted=yes] https://arkhimuttaqina.github.io/thinkpro-vantage/apt ./' | sudo tee /etc/apt/sources.list.d/thinkpro-vantage.list</span>
          <span class="ln cmd"><span class="prompt">$</span>sudo apt update &amp;&amp; sudo apt install thinkpro-vantage</span>
          <span class="ln"><br/></span>
          <span class="ln cmt"># Fedora</span>
          <span class="ln cmd"><span class="prompt">$</span>sudo dnf install ./thinkpro-vantage-*.rpm</span>
          <span class="ln"><br/></span>
          <span class="ln cmt"># Arch (build from the packaged PKGBUILD)</span>
          <span class="ln cmd"><span class="prompt">$</span>git clone https://github.com/ArkhiMuttaqina/thinkpro-vantage &amp;&amp; cd thinkpro-vantage/packaging/aur &amp;&amp; makepkg -si</span>
          <span class="cursor"></span>
        </div>
      </div>
    </div>
  </section>

  <!-- ============ CONTRIBUTE ============ -->
  <section class="contribute">
    <div class="hero-bg"><span class="blob b1"></span><span class="blob b2"></span></div>
    <div class="wrap" style="position:relative">
      <div class="section-head reveal">
        <span class="section-kicker">Contribute</span>
        <h2 class="section-title">Ship a better ThinkPad with us</h2>
        <p class="section-sub">Bug reports, kernel work, packaging, docs, translations — every contribution counts. The codebase is vanilla JS + Rust, no framework lock-in.</p>
      </div>
      <div class="contrib-actions reveal" style="--d:120ms">
        <a class="btn btn-brand" href="https://github.com/ArkhiMuttaqina/thinkpro-vantage/issues">Report an issue</a>
        <a class="btn btn-ghost" href="https://github.com/ArkhiMuttaqina/thinkpro-vantage">Open a pull request</a>
        <a class="btn btn-ghost" href="https://github.com/ArkhiMuttaqina/thinkpro-vantage/stargazers">Star the repo</a>
      </div>
      <p class="sponsor-note reveal" style="--d:200ms">LGPL-3.0, upstream-acknowledged. See the <a href="https://github.com/ArkhiMuttaqina/thinkpro-vantage">README</a> for the roadmap and revamp plan.</p>
    </div>
  </section>

  <footer class="landing-foot">ThinkPro Vantage · Linux-only ThinkPad companion · <a href="https://github.com/ArkhiMuttaqina/thinkpro-vantage">source on GitHub</a> · <a href="./guide/getting-started">docs</a></footer>

</div>

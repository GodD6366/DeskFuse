/* uniDesk 首页交互 */
(() => {
  "use strict";

  const prefersReduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---------- 导航滚动态 ---------- */
  const nav = document.getElementById("nav");
  const onScroll = () => nav.classList.toggle("scrolled", window.scrollY > 24);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  /* ---------- 滚动渐入 ---------- */
  const revealEls = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window && !prefersReduced) {
    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((e) => {
          if (e.isIntersecting) {
            e.target.classList.add("visible");
            io.unobserve(e.target);
          }
        });
      },
      { threshold: 0.12, rootMargin: "0px 0px -40px 0px" }
    );
    revealEls.forEach((el) => io.observe(el));
  } else {
    revealEls.forEach((el) => el.classList.add("visible"));
  }

  /* ---------- 卡片鼠标追光 ---------- */
  if (!prefersReduced) {
    document.querySelectorAll(".card").forEach((card) => {
      card.addEventListener("pointermove", (e) => {
        const r = card.getBoundingClientRect();
        card.style.setProperty("--mx", `${e.clientX - r.left}px`);
        card.style.setProperty("--my", `${e.clientY - r.top}px`);
      });
    });

    /* ---------- 截图 3D 倾斜 ---------- */
    document.querySelectorAll("[data-tilt]").forEach((wrap) => {
      const frame = wrap.querySelector(".shot-frame");
      if (!frame) return;
      wrap.addEventListener("pointermove", (e) => {
        const r = wrap.getBoundingClientRect();
        const rx = ((e.clientY - r.top) / r.height - 0.5) * -5;
        const ry = ((e.clientX - r.left) / r.width - 0.5) * 7;
        frame.style.transform = `rotateX(${rx}deg) rotateY(${ry}deg)`;
      });
      wrap.addEventListener("pointerleave", () => {
        frame.style.transform = "";
      });
    });
  }

  /* ---------- 界面展示 Tab ---------- */
  const shots = {
    clipboard: {
      src: "assets/shot-clipboard.jpg",
      alt: "uniDesk 剪贴板界面",
      caption: "剪贴板历史：文本、图片、文件一目了然，支持图片内容搜索",
    },
    devices: {
      src: "assets/shot-devices.jpg",
      alt: "uniDesk 设备界面",
      caption: "设备页：自动发现与手动配对，连接路由与传输活动全程可见",
    },
    sharing: {
      src: "assets/shot-sharing.jpg",
      alt: "uniDesk 分享与传输界面",
      caption: "分享与传输：局域网扫码即传，任务进度、排队与重试清晰可见",
    },
  };
  const shotImg = document.getElementById("shot-image");
  const shotCaption = document.getElementById("shot-caption");
  document.querySelectorAll(".shot-tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      const key = tab.dataset.shot;
      const data = shots[key];
      if (!data || tab.classList.contains("active")) return;
      document.querySelectorAll(".shot-tab").forEach((t) => t.classList.remove("active"));
      tab.classList.add("active");
      shotImg.classList.add("fading");
      setTimeout(() => {
        shotImg.src = data.src;
        shotImg.alt = data.alt;
        shotCaption.textContent = data.caption;
        shotImg.addEventListener("load", () => shotImg.classList.remove("fading"), { once: true });
      }, 180);
    });
  });

  /* ---------- 复制命令 ---------- */
  document.querySelectorAll(".copy-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const text = btn.dataset.copy || "";
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        const ta = document.createElement("textarea");
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        ta.remove();
      }
      const old = btn.textContent;
      btn.textContent = "已复制 ✓";
      btn.classList.add("copied");
      setTimeout(() => {
        btn.textContent = old;
        btn.classList.remove("copied");
      }, 1600);
    });
  });

  /* ---------- 从 GitHub 获取最新版本 ---------- */
  const RELEASES_API = "https://api.github.com/repos/GodD6366/unidesk-release/releases?per_page=5";
  const RELEASES_PAGE = "https://github.com/GodD6366/unidesk-release/releases";
  const versionBadge = document.getElementById("version-badge");
  const downloadBtn = document.getElementById("download-btn");
  const releaseLink = document.getElementById("release-link");

  fetch(RELEASES_API, { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((releases) => {
      if (!Array.isArray(releases) || releases.length === 0) return;
      const latest = releases[0];
      const tag = latest.tag_name || "";
      if (tag && versionBadge) {
        versionBadge.textContent = `最新版本 ${tag} · macOS arm64 / Intel · Windows x64`;
      }
      const dmg = (latest.assets || []).find((a) => /arm64.*\.dmg$/i.test(a.name || ""));
      const url = dmg ? dmg.browser_download_url : latest.html_url || RELEASES_PAGE;
      if (downloadBtn) downloadBtn.href = url;
      if (releaseLink) releaseLink.href = latest.html_url || RELEASES_PAGE;
    })
    .catch(() => {
      /* 离线或限流时保留静态回退 */
    });
})();

// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><a href="title-page.html">What Is Goose?</a></li><li class="chapter-item expanded affix "><a href="requirements.html">Requirements</a></li><li class="chapter-item expanded affix "><a href="glossary.html">Glossary</a></li><li class="chapter-item expanded "><a href="getting-started/overview.html"><strong aria-hidden="true">1.</strong> Getting Started</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="getting-started/creating.html"><strong aria-hidden="true">1.1.</strong> Creating A Load test</a></li><li class="chapter-item expanded "><a href="getting-started/validation.html"><strong aria-hidden="true">1.2.</strong> Validating Requests</a></li><li class="chapter-item expanded "><a href="getting-started/running.html"><strong aria-hidden="true">1.3.</strong> Running A Load Test</a></li><li class="chapter-item expanded "><a href="getting-started/runtime-options.html"><strong aria-hidden="true">1.4.</strong> Run-Time Options</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="getting-started/common.html"><strong aria-hidden="true">1.4.1.</strong> Common Options</a></li><li class="chapter-item expanded "><a href="getting-started/test-plan.html"><strong aria-hidden="true">1.4.2.</strong> Test Plan</a></li><li class="chapter-item expanded "><a href="getting-started/throttle.html"><strong aria-hidden="true">1.4.3.</strong> Throttle</a></li><li class="chapter-item expanded "><a href="getting-started/scenarios.html"><strong aria-hidden="true">1.4.4.</strong> Limiting Scenarios</a></li><li class="chapter-item expanded "><a href="getting-started/custom.html"><strong aria-hidden="true">1.4.5.</strong> Custom Options</a></li></ol></li><li class="chapter-item expanded "><a href="getting-started/metrics.html"><strong aria-hidden="true">1.5.</strong> Metrics</a></li><li class="chapter-item expanded "><a href="getting-started/tips.html"><strong aria-hidden="true">1.6.</strong> Tips</a></li></ol></li><li class="chapter-item expanded "><a href="logging/overview.html"><strong aria-hidden="true">2.</strong> Logging</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="logging/requests.html"><strong aria-hidden="true">2.1.</strong> Request Log</a></li><li class="chapter-item expanded "><a href="logging/transactions.html"><strong aria-hidden="true">2.2.</strong> Transaction Log</a></li><li class="chapter-item expanded "><a href="logging/scenarios.html"><strong aria-hidden="true">2.3.</strong> Scenario Log</a></li><li class="chapter-item expanded "><a href="logging/errors.html"><strong aria-hidden="true">2.4.</strong> Error Log</a></li><li class="chapter-item expanded "><a href="logging/debug.html"><strong aria-hidden="true">2.5.</strong> Debug Log</a></li></ol></li><li class="chapter-item expanded "><a href="controller/overview.html"><strong aria-hidden="true">3.</strong> Controllers</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="controller/telnet.html"><strong aria-hidden="true">3.1.</strong> Telnet Controller</a></li><li class="chapter-item expanded "><a href="controller/websocket.html"><strong aria-hidden="true">3.2.</strong> WebSocket Controller</a></li></ol></li><li class="chapter-item expanded "><a href="gaggle/overview.html"><strong aria-hidden="true">4.</strong> Gaggle: Distributed Load Test</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="gaggle/manager.html"><strong aria-hidden="true">4.1.</strong> Manager</a></li><li class="chapter-item expanded "><a href="gaggle/worker.html"><strong aria-hidden="true">4.2.</strong> Worker</a></li><li class="chapter-item expanded "><a href="gaggle/config.html"><strong aria-hidden="true">4.3.</strong> Configuration</a></li><li class="chapter-item expanded "><a href="gaggle/technical.html"><strong aria-hidden="true">4.4.</strong> Technical details</a></li></ol></li><li class="chapter-item expanded "><a href="coordinated-omission/overview.html"><strong aria-hidden="true">5.</strong> Coordinated Omission</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="coordinated-omission/mitigation.html"><strong aria-hidden="true">5.1.</strong> Mitigation</a></li><li class="chapter-item expanded "><a href="coordinated-omission/metrics.html"><strong aria-hidden="true">5.2.</strong> Metrics</a></li><li class="chapter-item expanded "><a href="coordinated-omission/examples.html"><strong aria-hidden="true">5.3.</strong> Practical Examples</a></li></ol></li><li class="chapter-item expanded "><a href="config/overview.html"><strong aria-hidden="true">6.</strong> Configuration</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="config/defaults.html"><strong aria-hidden="true">6.1.</strong> Defaults</a></li><li class="chapter-item expanded "><a href="config/scheduler.html"><strong aria-hidden="true">6.2.</strong> Scheduling Scenarios And Transactions</a></li><li class="chapter-item expanded "><a href="config/rustls.html"><strong aria-hidden="true">6.3.</strong> RustLS</a></li></ol></li><li class="chapter-item expanded "><a href="example/overview.html"><strong aria-hidden="true">7.</strong> Examples</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="example/simple.html"><strong aria-hidden="true">7.1.</strong> Simple</a></li><li class="chapter-item expanded "><a href="example/closure.html"><strong aria-hidden="true">7.2.</strong> Closure</a></li><li class="chapter-item expanded "><a href="example/session.html"><strong aria-hidden="true">7.3.</strong> Session</a></li><li class="chapter-item expanded "><a href="example/drupal-memcache.html"><strong aria-hidden="true">7.4.</strong> Drupal Memcache</a></li><li class="chapter-item expanded "><a href="example/umami.html"><strong aria-hidden="true">7.5.</strong> Umami</a></li></ol></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0].split("?")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);

// Adds a "Copy as Markdown" button to every rendered doc page, sourced from the raw .md file
// mdBook's own build already places alongside every .html page (see
// docs/scripts/copy-raw-markdown.sh) - never a re-rendered or re-serialized copy.
(function () {
    function mdPathFromHtmlPath(pathname) {
        if (!pathname.endsWith('.html')) return null;
        return pathname.slice(0, -'.html'.length) + '.md';
    }

    document.addEventListener('DOMContentLoaded', function () {
        var mdPath = mdPathFromHtmlPath(window.location.pathname);
        if (!mdPath) return;

        fetch(mdPath)
            .then(function (res) {
                return res.ok ? res.text() : null;
            })
            .catch(function () {
                return null;
            })
            .then(function (text) {
                if (!text) return;

                var button = document.createElement('button');
                button.type = 'button';
                button.className = 'djangors-copy-md';
                button.textContent = 'Copy as Markdown';
                button.addEventListener('click', function () {
                    navigator.clipboard.writeText(text).then(function () {
                        button.textContent = 'Copied';
                        setTimeout(function () {
                            button.textContent = 'Copy as Markdown';
                        }, 1400);
                    });
                });

                var content = document.querySelector('#mdbook-content main') || document.querySelector('#mdbook-content');
                if (content && content.firstChild) {
                    content.insertBefore(button, content.firstChild);
                } else if (content) {
                    content.appendChild(button);
                }
            });
    });
})();

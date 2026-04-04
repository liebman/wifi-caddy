
var messageEl = document.getElementById('message');
function showMessage(text, isError) {
    if (!messageEl) return;
    messageEl.textContent = text;
    messageEl.className = 'message ' + (isError ? 'error' : 'success') + ' show';
    setTimeout(function() { messageEl.classList.remove('show'); }, 5000);
}
var loaded = {};
function switchTab(page) {
    document.querySelectorAll('.config-tab').forEach(function(b) { b.classList.remove('active'); });
    document.querySelectorAll('.config-tab-panel').forEach(function(p) { p.style.display = 'none'; });
    var btn = document.querySelector('.config-tab[data-page="' + page + '"]');
    if (btn) btn.classList.add('active');
    var panel = document.getElementById('panel-' + page);
    if (panel) panel.style.display = '';
    if (!loaded[page]) {
        loaded[page] = true;
        var loadFn = window['loadConfig_' + page];
        if (loadFn) {
            loadFn().then(function() {
                panel.classList.add('loaded');
                panel.querySelectorAll('input,button').forEach(function(el) { el.disabled = false; });
                showMessage('Configuration loaded');
            }).catch(function(err) {
                var ov = document.getElementById('loading-' + page);
                if (ov) ov.textContent = 'Load failed: ' + err.message;
                var rb = panel.querySelector('.reloadBtn');
                if (rb) rb.disabled = false;
            });
        }
    }
}
document.querySelectorAll('.config-tab').forEach(function(btn) {
    btn.addEventListener('click', function() { switchTab(this.dataset.page); });
});
document.querySelectorAll('.config-tab-panel').forEach(function(panel) {
    panel.querySelectorAll('input,button').forEach(function(el) { el.disabled = true; });
});
document.querySelectorAll('.config-tab-panel form').forEach(function(form) {
    form.addEventListener('submit', function(e) {
        e.preventDefault();
        var page = this.id.replace('configForm-', '');
        var saveFn = window['saveConfig_' + page];
        if (saveFn) saveFn().then(function() { showMessage('Configuration saved'); })
            .catch(function(err) { showMessage('Save failed: ' + err.message, true); });
    });
    var rb = form.querySelector('.reloadBtn');
    if (rb) rb.addEventListener('click', function() {
        var page = this.closest('form').id.replace('configForm-', '');
        var loadFn = window['loadConfig_' + page];
        if (loadFn) loadFn().then(function() { showMessage('Configuration loaded'); })
            .catch(function(e) { showMessage('Load failed: ' + e.message, true); });
    });
});

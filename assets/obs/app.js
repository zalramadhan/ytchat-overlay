// Halaman chat untuk Browser Source OBS Studio.
// Menerima pesan lewat SSE dari aplikasi ytchat-overlay (localhost).
(function () {
    'use strict';

    var stage = document.getElementById('stage');
    var look = document.getElementById('look') || document.createElement('style');
    if (!look.id) {
        look.id = 'look';
        document.head.appendChild(look);
    }

    var settings = {
        fadeAfter: 0,
        fadeDuration: 3,
        maxMessages: 40
    };

    // Ganti shortcode emoji (`:nama:`) dengan gambar dari server; sisanya teks biasa.
    function richText(text, emotes) {
        var frag = document.createDocumentFragment();
        var re = /:([^:\s]+):/g;
        var last = 0;
        var match;

        while ((match = re.exec(text)) !== null) {
            var raw = match[1];
            var key = raw.replace(/^_+|_+$/g, '');
            var url = emotes && (emotes[key] || emotes[raw]);
            if (!url) {
                continue;
            }
            if (match.index > last) {
                frag.appendChild(document.createTextNode(text.slice(last, match.index)));
            }
            var img = document.createElement('img');
            img.className = 'emoji';
            img.src = url;
            img.alt = raw;
            frag.appendChild(img);
            last = re.lastIndex;
        }

        if (last < text.length) {
            frag.appendChild(document.createTextNode(text.slice(last)));
        }
        return frag;
    }

    function trimStage() {
        while (stage.children.length > settings.maxMessages) {
            stage.removeChild(stage.firstElementChild);
        }
    }

    function scheduleFade(row) {
        if (!(settings.fadeAfter > 0)) {
            return;
        }
        window.setTimeout(function () {
            if (!row.isConnected) {
                return;
            }
            row.classList.add('fading');
            window.setTimeout(function () {
                if (row.parentNode) {
                    row.parentNode.removeChild(row);
                }
            }, Math.max(settings.fadeDuration, 0.1) * 1000);
        }, settings.fadeAfter * 1000);
    }

    function addMessage(msg) {
        var row = document.createElement('div');
        row.className = 'msg';

        if (msg.kind === 'superchat' || msg.kind === 'sticker') {
            row.classList.add('sc' + Math.min(Math.max(msg.tier || 1, 1), 7));
        }
        if (msg.kind === 'member' || msg.member) {
            row.classList.add('member');
        }
        if (msg.kind === 'system') {
            row.classList.add('system');
        }
        if (msg.owner) {
            row.classList.add('owner');
        }
        if (msg.mod) {
            row.classList.add('mod');
        }

        if (msg.avatar) {
            var avatar = document.createElement('img');
            avatar.className = 'avatar';
            avatar.src = msg.avatar;
            avatar.alt = '';
            row.appendChild(avatar);
        }

        var col = document.createElement('div');
        col.className = 'col';

        if (msg.author) {
            var head = document.createElement('div');
            head.className = 'head';

            var name = document.createElement('span');
            name.className = 'author';
            name.textContent = msg.author + (msg.verified ? ' ✔' : '');
            head.appendChild(name);

            if (msg.amount) {
                var amount = document.createElement('span');
                amount.className = 'amount';
                amount.textContent = msg.amount;
                head.appendChild(amount);
            }
            col.appendChild(head);
        }

        var body = document.createElement('div');
        body.className = 'body';
        if (msg.kind === 'system') {
            body.textContent = msg.text;
        } else {
            body.appendChild(richText(msg.text || '', msg.emotes));
        }
        col.appendChild(body);

        row.appendChild(col);
        stage.appendChild(row);
        trimStage();
        scheduleFade(row);
    }

    function applyStyle(data) {
        if (typeof data.css === 'string') {
            look.textContent = data.css;
        }
        if (typeof data.fadeAfter === 'number') {
            settings.fadeAfter = data.fadeAfter;
        }
        if (typeof data.fadeDuration === 'number') {
            settings.fadeDuration = data.fadeDuration;
        }
        if (typeof data.maxMessages === 'number') {
            settings.maxMessages = Math.max(data.maxMessages, 1);
            trimStage();
        }
    }

    function connect() {
        var source = new EventSource('/events');

        source.onmessage = function (event) {
            var data;
            try {
                data = JSON.parse(event.data);
            } catch (e) {
                return;
            }
            if (data.type === 'style') {
                applyStyle(data);
            } else if (data.type === 'msg') {
                addMessage(data);
            } else if (data.type === 'clear') {
                stage.textContent = '';
            }
        };

        // EventSource menyambung ulang sendiri; kalau server dimatikan, halaman
        // tetap steril dan mencoba lagi sampai server hidup kembali.
        source.onerror = function () {
            if (source.readyState === EventSource.CLOSED) {
                window.setTimeout(connect, 3000);
            }
        };
    }

    connect();
})();

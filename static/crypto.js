function bytesToB64(bytes) {
    return btoa(String.fromCharCode(...bytes));
}

function b64ToBytes(b64) {
    return Uint8Array.from(atob(b64), c => c.charCodeAt(0));
}

async function importRawKey(rawKeyB64) {
    return crypto.subtle.importKey(
        'raw',
        b64ToBytes(rawKeyB64),
        { name: 'AES-GCM' },
        false,
        ['encrypt', 'decrypt']
    );
}

async function deriveKeyFromPassword(password, saltB64) {
    const baseKey = await crypto.subtle.importKey(
        'raw',
        new TextEncoder().encode(password),
        'PBKDF2',
        false,
        ['deriveKey']
    );
    return crypto.subtle.deriveKey(
        { name: 'PBKDF2', salt: b64ToBytes(saltB64), iterations: 600000, hash: 'SHA-256' },
        baseKey,
        { name: 'AES-GCM', length: 256 },
        false,
        ['encrypt', 'decrypt']
    );
}

function generateKey() {
    return bytesToB64(crypto.getRandomValues(new Uint8Array(32)));
}

function generateSalt() {
    return bytesToB64(crypto.getRandomValues(new Uint8Array(16)));
}

async function encryptWithKey(cryptoKey, payload) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encoded = new TextEncoder().encode(JSON.stringify(payload));
    const ciphertext = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, cryptoKey, encoded);
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv);
    combined.set(new Uint8Array(ciphertext), iv.length);
    return bytesToB64(combined);
}

async function decryptWithKey(cryptoKey, encrypted) {
    const combined = b64ToBytes(encrypted);
    const iv = combined.slice(0, 12);
    const ciphertext = combined.slice(12);
    const decrypted = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, cryptoKey, ciphertext);
    return JSON.parse(new TextDecoder().decode(decrypted));
}

// password 为空/undefined → 链接模式（随机密钥放 fragment）
// password 非空 → 密码模式（密钥由密码 + salt 派生，链接不含密钥）
async function createShare(payload, password) {
    let key = null;
    let kdfSalt = null;
    let cryptoKey;

    if (password) {
        kdfSalt = generateSalt();
        cryptoKey = await deriveKeyFromPassword(password, kdfSalt);
    } else {
        key = generateKey();
        cryptoKey = await importRawKey(key);
    }

    const encryptedPayload = await encryptWithKey(cryptoKey, payload);

    const res = await fetch('/api/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ encrypted_payload: encryptedPayload, kdf_salt: kdfSalt })
    });

    if (!res.ok) {
        const text = await res.text();
        throw new Error(text || '创建失败');
    }

    const { slug } = await res.json();
    return { slug, key, passwordProtected: !!password };
}

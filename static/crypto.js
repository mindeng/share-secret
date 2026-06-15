async function deriveKey(rawKey) {
    const keyData = Uint8Array.from(atob(rawKey), c => c.charCodeAt(0));
    return await crypto.subtle.importKey(
        'raw',
        keyData,
        { name: 'AES-GCM' },
        false,
        ['encrypt', 'decrypt']
    );
}

function generateKey() {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    return btoa(String.fromCharCode(...bytes));
}

async function encryptPayload(key, payload) {
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encoded = new TextEncoder().encode(JSON.stringify(payload));
    const cryptoKey = await deriveKey(key);
    const ciphertext = await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv },
        cryptoKey,
        encoded
    );
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv);
    combined.set(new Uint8Array(ciphertext), iv.length);
    return btoa(String.fromCharCode(...combined));
}

async function decryptPayload(key, encrypted) {
    const combined = Uint8Array.from(atob(encrypted), c => c.charCodeAt(0));
    const iv = combined.slice(0, 12);
    const ciphertext = combined.slice(12);
    const cryptoKey = await deriveKey(key);
    const decrypted = await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv },
        cryptoKey,
        ciphertext
    );
    return JSON.parse(new TextDecoder().decode(decrypted));
}

async function createShare(payload) {
    const key = generateKey();
    const encryptedPayload = await encryptPayload(key, payload);

    const res = await fetch('/api/shares', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ encrypted_payload: encryptedPayload })
    });

    if (!res.ok) {
        const text = await res.text();
        throw new Error(text || '创建失败');
    }

    const { slug } = await res.json();
    return { slug, key, encryptedPayload };
}

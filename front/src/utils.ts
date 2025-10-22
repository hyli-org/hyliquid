// TODO: finally put this somewhere common
export const encodeToHex = (data: Uint8Array | number[]): string => {
    return (() => {
        if (data instanceof Uint8Array) {
            return Array.from(data);
        } else if (Array.isArray(data)) {
            return data;
        } else {
            throw new TypeError("Unsupported data type for encodeToHex");
        }
    })()
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join("");
};

export const normalizeHexLike = (value: string): string => {
    const trimmed = value.trim();
    if (!trimmed.startsWith("0x")) {
        return trimmed.toLowerCase();
    }
    return `0x${trimmed.slice(2).toLowerCase()}`;
};

export const isHexAddress = (value: string): boolean => /^0x[a-fA-F0-9]{40}$/.test(value.trim());

export const requireHexAddress = (
            value: string | undefined | null,
            errorMessage: string,
            missingMessage?: string,
        ): string => {
    const trimmed = value?.trim() ?? "";
    if (!trimmed) {
        throw new Error(missingMessage ?? errorMessage);
    }
    if (!isHexAddress(trimmed)) {
        throw new Error(errorMessage);
    }
    return normalizeHexLike(trimmed);
};
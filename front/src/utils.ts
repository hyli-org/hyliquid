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

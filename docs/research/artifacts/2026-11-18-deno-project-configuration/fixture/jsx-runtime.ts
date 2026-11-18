export namespace JSX { export interface IntrinsicElements { article: {children?: unknown}; } }
export const Fragment = "fragment";
export function jsx(type: string, props: unknown) { return {type,props}; }
export const jsxs = jsx;

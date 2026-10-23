/** 静态资源路径（Vite base 为 './'，拼接后相对当前页面解析）。 */
export const asset = (path: string) => `${import.meta.env.BASE_URL}${path}`

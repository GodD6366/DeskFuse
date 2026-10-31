export interface MenuItem {
  label?: string
  /** Icon.vue 中的名称。 */
  icon?: string
  shortcut?: string
  disabled?: boolean
  checked?: boolean
  danger?: boolean
  separator?: boolean
  children?: MenuItem[]
  /** 子菜单异步加载；返回后替换 children。 */
  load?: () => Promise<MenuItem[]>
  run?: () => void
}

export const separator: MenuItem = { separator: true }

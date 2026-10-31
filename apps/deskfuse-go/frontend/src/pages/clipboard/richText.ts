export interface RichRun {
  text: string
  bold?: boolean
  italic?: boolean
  underline?: boolean
  strike?: boolean
}

// 只允许编辑器需要的文字与四种样式；不把粘贴内容中的 HTML、链接或资源交给原生侧。
export function writeRuns(root: HTMLElement, runs: RichRun[], fallback: string): void {
  root.replaceChildren()
  for (const run of runs.length ? runs : [{ text: fallback }]) {
    const span = document.createElement('span')
    span.textContent = run.text
    if (run.bold) span.style.fontWeight = '600'
    if (run.italic) span.style.fontStyle = 'italic'
    if (run.underline || run.strike) {
      span.style.textDecoration = [run.underline && 'underline', run.strike && 'line-through'].filter(Boolean).join(' ')
    }
    root.append(span)
  }
}

export function readRuns(root: HTMLElement): RichRun[] {
  const runs: RichRun[] = []
  const append = (text: string, style: Omit<RichRun, 'text'>) => {
    if (!text) return
    const previous = runs.at(-1)
    if (previous && previous.bold === style.bold && previous.italic === style.italic
      && previous.underline === style.underline && previous.strike === style.strike) previous.text += text
    else runs.push({ text, ...style })
  }
  const visit = (node: Node, inherited: Omit<RichRun, 'text'>) => {
    if (node.nodeType === Node.TEXT_NODE) { append(node.textContent ?? '', inherited); return }
    if (!(node instanceof HTMLElement)) return
    if (['SCRIPT', 'STYLE', 'IMG', 'IFRAME', 'OBJECT'].includes(node.tagName)) return
    if (node.tagName === 'BR') { append('\n', inherited); return }
    const block = ['DIV', 'P', 'LI', 'PRE', 'BLOCKQUOTE'].includes(node.tagName)
    if (block && runs.length && !runs.at(-1)!.text.endsWith('\n')) append('\n', inherited)
    const css = getComputedStyle(node)
    const style = {
      bold: Number.parseInt(css.fontWeight) >= 600 || css.fontWeight === 'bold',
      italic: css.fontStyle === 'italic',
      underline: inherited.underline || css.textDecorationLine.includes('underline'),
      strike: inherited.strike || css.textDecorationLine.includes('line-through'),
    }
    for (const child of node.childNodes) visit(child, style)
    // 块间换行由下一个块的开头插入；末尾块不额外添加换行。
  }
  for (const child of root.childNodes) visit(child, {})
  return runs
}

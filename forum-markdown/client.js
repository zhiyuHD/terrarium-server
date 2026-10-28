import MarkdownIt from 'markdown-it';
const markdown = new MarkdownIt({ html: false, breaks: true, linkify: false, maxNesting: 20 });
const validateLink = markdown.validateLink;
markdown.validateLink = href => {
  try { return validateLink(href) && ['http:', 'https:', 'mailto:'].includes(new URL(href, document.baseURI).protocol); }
  catch { return false; }
};
const link = markdown.renderer.rules.link_open || ((tokens, index, options, env, self) => self.renderToken(tokens, index, options));
markdown.renderer.rules.link_open = (tokens, index, options, env, self) => {
  const token = tokens[index], href = token.attrGet('href');
  if (href && new URL(href, document.baseURI).origin !== location.origin) {
    token.attrSet('target', '_blank'); token.attrSet('rel', 'noopener noreferrer');
  }
  return link(tokens, index, options, env, self);
};
const image = markdown.renderer.rules.image;
markdown.renderer.rules.image = (tokens, index, options, env, self) => {
  const token = tokens[index];
  const protocol = new URL(token.attrGet('src'), document.baseURI).protocol;
  if (!['http:', 'https:'].includes(protocol)) return markdown.utils.escapeHtml(token.content);
  token.attrSet('loading', 'lazy'); token.attrSet('referrerpolicy', 'no-referrer');
  return image(tokens, index, options, env, self);
};
export function render(body) {
  const content = document.createElement('div'); content.className = 'markdown';
  content.innerHTML = markdown.render(String(body ?? ''));
  return content;
}

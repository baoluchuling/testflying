export function NotFoundPage() {
  function returnHome() {
    history.pushState({ adminRoute: 'dashboard' }, '', '/admin');
  }

  return (
    <section className="panel not-found-page">
      <p className="eyebrow">404</p>
      <h2>页面不存在</h2>
      <p>该地址已被移除或从未存在，请从当前后台导航重新进入。</p>
      <button className="button primary" type="button" onClick={returnHome}>
        返回总览
      </button>
    </section>
  );
}

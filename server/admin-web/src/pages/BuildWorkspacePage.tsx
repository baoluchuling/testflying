import { BuildRunnersPage } from './BuildRunnersPage';
import { BuildAppsPage } from './BuildAppsPage';
import { BuildHistoryPage } from './BuildHistoryPage';
import type { BuildView } from '../app/routes';

const views: Array<{ key: BuildView; label: string; path: string }> = [
  { key: 'apps', label: '应用构建', path: '/admin/builds/apps' },
  { key: 'history', label: '构建历史', path: '/admin/builds/history' },
  { key: 'runners', label: '构建节点', path: '/admin/builds/runners' }
];

export function BuildWorkspacePage({ view }: { view: BuildView }) {
  function navigate(path: string) {
    history.pushState({ adminRoute: 'builds' }, '', path);
  }

  return (
    <div className="section-workspace" data-build-workspace>
      <nav className="secondary-nav" aria-label="构建导航">
        {views.map((item) => (
          <button
            key={item.key}
            className={view === item.key ? 'secondary-nav-item active' : 'secondary-nav-item'}
            type="button"
            onClick={() => navigate(item.path)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      {view === 'apps' ? <BuildAppsPage /> : null}
      {view === 'history' ? <BuildHistoryPage /> : null}
      {view === 'runners' ? <BuildRunnersPage /> : null}
    </div>
  );
}

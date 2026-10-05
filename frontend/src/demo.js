import { ROOT_ID } from './api';
const date = offset => Math.floor(Date.now() / 1000) - offset * 3600;
export function demoFiles() {
  const file = (id, name, size, hours, extra = {}) => ({ file_id: id, file_name: name, file_size: size, file_owner: 'demo', parent_id: ROOT_ID, file_created_at: date(hours + 20), file_updated_at: date(hours), is_directory: false, ...extra });
  return [
    file('folder-work', '工作文档', 0, 2, { is_directory: true }),
    file('folder-photo', '生活影像', 0, 5, { is_directory: true }),
    file('folder-design', '设计素材', 0, 8, { is_directory: true }),
    file('f1', '品牌设计指南.pdf', 2480000, 1, { featured: 'brand' }),
    file('f2', '山野之间.jpg', 5860000, 3, { featured: 'landscape' }),
    file('f3', '2026 项目计划.docx', 128000, 4, { featured: 'plan' }),
    file('f4', '秋日散步.mp4', 42800000, 24),
    file('f5', '灵感素材.zip', 18600000, 48),
    file('f6', '周末歌单.mp3', 7200000, 70),
    file('child1', '会议纪要.md', 4600, 6, { parent_id: 'folder-work', text: '# 会议纪要\n\n本周事项\n- 完成网盘前端设计\n- 整理项目文档\n- 对接文件管理接口' }),
    file('child2', '旅途随记.txt', 2300, 9, { parent_id: 'folder-photo', text: '慢慢走，看看沿途的风景。\n\n把每一个值得记住的瞬间，好好收藏。' }),
    file('child3', '色彩参考.txt', 1200, 12, { parent_id: 'folder-design', text: '森林绿 #245C4C\n暖白 #F7F9F8\n淡绿 #E3EEE8' }),
  ];
}

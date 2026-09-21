import { useState, useEffect, useCallback, useRef } from 'react';
import { Layout, Tree, Table, Button, Input, Space, Breadcrumb, Dropdown, message, Modal, Tooltip, Menu } from 'antd';
import {
  FolderOutlined,
  FolderOpenOutlined,
  ImportOutlined,
  ExportOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderAddOutlined,
  HomeOutlined,
  AppstoreOutlined,
  UnorderedListOutlined,
  ReloadOutlined,
} from '@ant-design/icons';
import { 
  FaFilePdf, 
  FaFileWord, 
  FaFileExcel, 
  FaFilePowerpoint, 
  FaFileArchive, 
  FaFileCode, 
  FaFileImage, 
  FaFileVideo, 
  FaFileAudio,
  FaFileAlt,
  FaFile,
  FaMarkdown,
  FaWindows
} from 'react-icons/fa';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { DataNode } from 'antd/es/tree';
import type { MenuProps } from 'antd';
import './App.css';

interface FileInfo {
  file_id: number;
  name: string;
  ext: string | null;
  mime_type: string | null;
  size_bytes: number;
  duration_sec: number | null;
  width: number | null;
  height: number | null;
  category: string | null;
  thumbnail_path: string | null;
  created_at: number;
  updated_at: number;
}

interface FolderInfo {
  folder_id: number;
  name: string;
  parent_id: number | null;
  created_at: number;
}

interface ImportResult {
  success_count: number;
  fail_count: number;
  errors: string[];
}

interface FolderContentsItem {
  id: number;
  name: string;
  is_folder: boolean;
  size_bytes: number;
  category: string | null;
  ext: string | null;
  thumbnail_path: string | null;
  updated_at: number;
}

interface FolderContents {
  items: FolderContentsItem[];
}

interface BreadcrumbItem {
  id: number;
  name: string;
}

interface BatchResult {
  success_count: number;
  fail_count: number;
  errors: string[];
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

function formatTime(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

function getFileIcon(category: string | null, ext?: string | null) {
  const extension = ext?.toLowerCase();

  // 根据扩展名显示更精确的图标（不设置 fontSize，由父容器控制大小）
  if (extension) {
    switch (extension) {
      // 文档
      case 'pdf':
        return <FaFilePdf style={{ color: '#ff4d4f' }} />;
      case 'doc':
      case 'docx':
        return <FaFileWord style={{ color: '#2b579a' }} />;
      case 'xls':
      case 'xlsx':
        return <FaFileExcel style={{ color: '#217346' }} />;
      case 'ppt':
      case 'pptx':
        return <FaFilePowerpoint style={{ color: '#d24726' }} />;
      case 'txt':
        return <FaFileAlt style={{ color: '#8c8c8c' }} />;
      case 'md':
        return <FaMarkdown style={{ color: '#083fa1' }} />;

      // 压缩包
      case 'zip':
      case 'rar':
      case '7z':
      case 'tar':
      case 'gz':
        return <FaFileArchive style={{ color: '#faad14' }} />;

      // 代码
      case 'js':
      case 'ts':
      case 'jsx':
      case 'tsx':
      case 'py':
      case 'java':
      case 'cpp':
      case 'c':
      case 'h':
      case 'cs':
      case 'go':
      case 'rs':
      case 'html':
      case 'css':
      case 'json':
      case 'xml':
      case 'yaml':
      case 'yml':
        return <FaFileCode style={{ color: '#1890ff' }} />;

      // 可执行文件
      case 'exe':
      case 'msi':
        return <FaWindows style={{ color: '#0078d4' }} />;

      // 图片
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
      case 'bmp':
      case 'webp':
      case 'svg':
      case 'ico':
        return <FaFileImage style={{ color: '#52c41a' }} />;

      // 视频
      case 'mp4':
      case 'avi':
      case 'mkv':
      case 'mov':
      case 'wmv':
      case 'flv':
      case 'webm':
        return <FaFileVideo style={{ color: '#722ed1' }} />;

      // 音频
      case 'mp3':
      case 'wav':
      case 'flac':
      case 'aac':
      case 'ogg':
      case 'wma':
        return <FaFileAudio style={{ color: '#eb2f96' }} />;
    }
  }

  // 根据类别显示默认图标
  switch (category) {
    case 'image': return <FaFileImage style={{ color: '#52c41a' }} />;
    case 'video_short':
    case 'video_long': return <FaFileVideo style={{ color: '#722ed1' }} />;
    case 'doc': return <FaFileAlt style={{ color: '#8c8c8c' }} />;
    default: return <FaFile style={{ color: '#8c8c8c' }} />;
  }
}

function App() {
  const [folders, setFolders] = useState<FolderInfo[]>([]);
  const [items, setItems] = useState<FolderContentsItem[]>([]);
  const [currentFolderId, setCurrentFolderId] = useState<number>(1);
  const [selectedFileIds, setSelectedFileIds] = useState<number[]>([]);
  const [searchKeyword, setSearchKeyword] = useState('');
  const [viewMode, setViewMode] = useState<'list' | 'grid'>('list');
  const [, setLoading] = useState(false);
  const [breadcrumb, setBreadcrumb] = useState<{ id: number; name: string }[]>([
    { id: 1, name: '根目录' },
  ]);
  const [renameModalOpen, setRenameModalOpen] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [renameFileId, setRenameFileId] = useState<number | null>(null);
  const [newFolderModalOpen, setNewFolderModalOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');
  const [expandedKeys, setExpandedKeys] = useState<React.Key[]>([1]); // 默认展开根目录
  const currentFolders = items.filter(i => i.is_folder);
  const files = items.filter(i => !i.is_folder);

  // 单击停顿再单击 = 重命名（用 ref 避免闭包陈旧值问题）
  const lastClickTimeRef = useRef<number>(0);
  const lastClickIdRef = useRef<number | null>(null);
  const [inlineRenameId, setInlineRenameId] = useState<number | null>(null); // 正数=file_id, 负数=-folder_id
  const [inlineRenameValue, setInlineRenameValue] = useState('');

  // 拖拽选择框（用 ref 存储起始点，避免闭包陈旧值）
  const [isSelecting, setIsSelecting] = useState(false);
  const [selectionRect, setSelectionRect] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const selectStartRef = useRef<{ x: number; y: number } | null>(null);
  const justFinishedSelectionRef = useRef<boolean>(false); // 标记是否刚完成框选

  // 自定义拖拽移动（替代 HTML5 拖拽 API，解决 WebView2 兼容性问题）
  const [isCustomDragging, setIsCustomDragging] = useState(false);
  const [customDragMouse, setCustomDragMouse] = useState<{ x: number; y: number } | null>(null);
  const [sidebarHoverFolderId, setSidebarHoverFolderId] = useState<number | null>(null);
  const mouseDownInfoRef = useRef<{ x: number; y: number; target: HTMLElement; isGridItem: boolean; isFileRow: boolean } | null>(null);
  const customDragIdsRef = useRef<number[]>([]);

  // 右键菜单
  const [contextMenuRecord, setContextMenuRecord] = useState<any>(null);
  const [contextMenuPos, setContextMenuPos] = useState<{ x: number; y: number } | null>(null);

  const loadFolders = useCallback(async () => {
    try {
      const result = await invoke<FolderInfo[]>('get_folders');
      setFolders(result);
    } catch (e) {
      console.error('Failed to load folders:', e);
    }
  }, []);

  const loadFolderContents = useCallback(async (folderId: number) => {
    setLoading(true);
    try {
      const result = await invoke<FolderContents>('get_folder_contents', { folderId });
      setItems(result.items);
    } catch (e) {
      console.error('Failed to load folder contents:', e);
      message.error('加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadFolders();
    loadFolderContents(currentFolderId);
  }, [currentFolderId, loadFolders, loadFolderContents]);

  // 全局鼠标事件监听，确保拖拽和选择框在鼠标移出内容区域后仍能工作
  useEffect(() => {
    const handleGlobalMouseMove = (e: MouseEvent) => {
      if (isCustomDragging) {
        setCustomDragMouse({ x: e.clientX, y: e.clientY });
        // 检测鼠标是否悬停在侧边栏的文件夹上
        const treeNodes = document.querySelectorAll('.ant-tree-treenode');
        let foundFolderId: number | null = null;
        treeNodes.forEach(node => {
          const rect = node.getBoundingClientRect();
          if (e.clientX >= rect.left && e.clientX <= rect.right &&
              e.clientY >= rect.top && e.clientY <= rect.bottom) {
            // 通过 data-folder-id 属性精确匹配文件夹
            const folderIdAttr = node.getAttribute('data-folder-id') || 
                                 node.querySelector('[data-folder-id]')?.getAttribute('data-folder-id');
            if (folderIdAttr) {
              foundFolderId = parseInt(folderIdAttr);
            }
          }
        });
        setSidebarHoverFolderId(foundFolderId);
      }
    };

    const handleGlobalMouseUp = () => {
      if (isCustomDragging) {
        if (sidebarHoverFolderId !== null) {
          const ids = customDragIdsRef.current;
          if (ids.length > 0) {
            invoke<BatchResult>('batch_move', { ids, targetFolderId: sidebarHoverFolderId })
              .then(result => {
                if (result.fail_count > 0) {
                  message.warning(`移动完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
                } else {
                  message.success(`成功移动 ${result.success_count} 个项目`);
                }
                loadFolderContents(currentFolderId);
                loadFolders();
              })
              .catch(err => message.error('移动失败: ' + err));
          }
        }
        setIsCustomDragging(false);
        setCustomDragMouse(null);
        setSidebarHoverFolderId(null);
        customDragIdsRef.current = [];
        mouseDownInfoRef.current = null;
      }
    };

    window.addEventListener('mousemove', handleGlobalMouseMove);
    window.addEventListener('mouseup', handleGlobalMouseUp);

    return () => {
      window.removeEventListener('mousemove', handleGlobalMouseMove);
      window.removeEventListener('mouseup', handleGlobalMouseUp);
    };
  }, [isCustomDragging, sidebarHoverFolderId, folders, currentFolderId]);

  // Build tree data from folders
  const buildTreeData = (folders: FolderInfo[]): DataNode[] => {
    const folderMap = new Map<number, FolderInfo[]>();
    folders.forEach(f => {
      const pid = f.parent_id ?? 0;
      if (!folderMap.has(pid)) folderMap.set(pid, []);
      folderMap.get(pid)!.push(f);
    });

    const buildNode = (parentId: number): DataNode[] => {
      const children = folderMap.get(parentId) || [];
      return children.map(f => ({
        key: f.folder_id,
        title: f.name,
        icon: currentFolderId === f.folder_id ? <FolderOpenOutlined /> : <FolderOutlined />,
        children: buildNode(f.folder_id),
        // 添加自定义属性用于拖拽检测
        'data-folder-id': f.folder_id,
      }));
    };

    return buildNode(0);
  };

  const handleFolderSelect = (selectedKeys: React.Key[]) => {
    if (selectedKeys.length > 0) {
      const folderId = selectedKeys[0] as number;
      enterFolder(folderId);
    }
  };

  // 进入文件夹
  const enterFolder = async (folderId: number) => {
    setCurrentFolderId(folderId);
    setSelectedFileIds([]);
    setSearchKeyword('');
    setExpandedKeys(prev => prev.includes(folderId) ? prev : [...prev, folderId]);

    // 后端负责面包屑路径构建
    try {
      const path = await invoke<BreadcrumbItem[]>('get_breadcrumb_path', { folderId });
      setBreadcrumb(path);
    } catch (e) {
      console.error('Failed to load breadcrumb:', e);
    }
  };

  const handleImportFiles = async () => {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        const result = await invoke<ImportResult>('import_files', {
          filePaths: paths,
          folderId: currentFolderId,
        });
        if (result.fail_count > 0) {
          message.warning(`导入完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
        } else {
          message.success(`成功导入 ${result.success_count} 个文件`);
        }
        loadFolderContents(currentFolderId);
        loadFolders();
      }
    } catch (e) {
      message.error('导入失败: ' + e);
    }
  };

  const handleImportFolders = async () => {
    try {
      const selected = await open({
        multiple: true,
        directory: true,
      });
      if (selected) {
        const paths = Array.isArray(selected) ? selected : [selected];
        console.log('Importing folders:', paths);
        const result = await invoke<ImportResult>('import_files', {
          filePaths: paths,
          folderId: currentFolderId,
        });
        console.log('Import result:', result);
        if (result.fail_count > 0) {
          message.warning(`导入完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
          if (result.errors.length > 0) {
            console.error('Import errors:', result.errors);
          }
        } else {
          message.success(`成功导入 ${result.success_count} 个文件，请查看左侧目录树`);
        }
        // Refresh both files and folders
        await loadFolders();
        await loadFolderContents(currentFolderId);
      }
    } catch (e) {
      console.error('Import failed:', e);
      message.error('导入失败: ' + e);
    }
  };

  const handleExport = async () => {
    if (selectedFileIds.length === 0) {
      message.warning('请先选择要导出的文件或文件夹');
      return;
    }
    try {
      const targetDir = await open({
        multiple: false,
        directory: true,
      });
      if (targetDir) {
        const dir = Array.isArray(targetDir) ? targetDir[0] : targetDir;
        await invoke<string[]>('batch_export', {
          ids: selectedFileIds,
          targetDir: dir,
        });
        message.success(`成功导出 ${selectedFileIds.length} 个项目`);
      }
    } catch (e) {
      message.error('导出失败: ' + e);
    }
  };

  const handleDelete = async (ids?: number[]) => {
    const targetIds = ids || selectedFileIds;
    if (targetIds.length === 0) return;
    try {
      const result = await invoke<BatchResult>('batch_delete', { ids: targetIds });
      if (result.fail_count > 0) {
        message.warning(`删除完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
      } else {
        message.success(`已删除 ${result.success_count} 个项目`);
      }
      setSelectedFileIds([]);
      loadFolderContents(currentFolderId);
      loadFolders();
    } catch (e) {
      message.error('删除失败: ' + e);
    }
  };

  const handleOpenFile = async (fileId: number) => {
    try {
      await invoke('open_file', { fileId });
    } catch (e) {
      message.error('打开文件失败: ' + e);
    }
  };

  const handleSearch = async () => {
    if (!searchKeyword.trim()) {
      loadFolderContents(currentFolderId);
      return;
    }
    setLoading(true);
    try {
      const result = await invoke<FileInfo[]>('search_files', { keyword: searchKeyword });
      setItems(result.map(f => ({
        id: f.file_id,
        name: f.name,
        is_folder: false,
        size_bytes: f.size_bytes,
        category: f.category,
        ext: f.ext,
        thumbnail_path: f.thumbnail_path,
        updated_at: f.updated_at,
      })));
    } catch (e) {
      console.error('Search failed:', e);
    } finally {
      setLoading(false);
    }
  };

  const handleCreateFolder = async () => {
    if (!newFolderName.trim()) return;
    try {
      await invoke('create_folder', {
        name: newFolderName,
        parentId: currentFolderId,
      });
      setNewFolderModalOpen(false);
      setNewFolderName('');
      loadFolders();
      message.success('文件夹创建成功');
    } catch (e) {
      message.error('创建文件夹失败: ' + e);
    }
  };

  const handleRename = async () => {
    if (!renameValue.trim() || renameFileId === null) return;
    try {
      await invoke('rename_file', { fileId: renameFileId, newName: renameValue });
      setRenameModalOpen(false);
      setRenameValue('');
      setRenameFileId(null);
      loadFolderContents(currentFolderId);
      message.success('重命名成功');
    } catch (e) {
      message.error('重命名失败: ' + e);
    }
  };

  // 单击停顿再单击 = 重命名（Windows风格）
  const handleClickForRename = (id: number, name: string) => {
    const now = Date.now();
    const lastTime = lastClickTimeRef.current;
    const lastId = lastClickIdRef.current;
    const elapsed = now - lastTime;
    
    if (lastId === id && elapsed >= 300 && elapsed < 1500) {
      // 停顿后再点击，进入重命名模式
      setInlineRenameId(id);
      setInlineRenameValue(name);
      lastClickIdRef.current = null;
      lastClickTimeRef.current = 0;
    } else if (lastId === id && elapsed < 300) {
      // 双击，不重置 lastClickTime，保持原值
    } else {
      // 首次点击或点击了不同项目
      lastClickIdRef.current = id;
      lastClickTimeRef.current = now;
    }
  };

  // 提交行内重命名
  const handleInlineRenameSubmit = async () => {
    if (inlineRenameId === null || !inlineRenameValue.trim()) {
      setInlineRenameId(null);
      return;
    }
    try {
      if (inlineRenameId > 0) {
        // 文件重命名
        await invoke('rename_file', { fileId: inlineRenameId, newName: inlineRenameValue });
      } else {
        // 文件夹重命名
        await invoke('rename_folder', { folderId: -inlineRenameId, newName: inlineRenameValue });
      }
      message.success('重命名成功');
      loadFolderContents(currentFolderId);
      loadFolders();
    } catch (e) {
      message.error('重命名失败: ' + e);
    }
    setInlineRenameId(null);
  };

  // 统一鼠标事件处理：选择框 + 自定义拖拽
  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    const gridItem = target.closest('.grid-item') as HTMLElement;
    const fileRow = target.closest('.file-row') as HTMLElement;

    if (gridItem) {
      // 点击了图标视图的项目
      const id = parseInt(gridItem.getAttribute('data-id') || '0');
      // 如果点击的是已选中项目，准备可能的拖拽
      if (selectedFileIds.includes(id)) {
        mouseDownInfoRef.current = { x: e.clientX, y: e.clientY, target: gridItem, isGridItem: true, isFileRow: false };
        customDragIdsRef.current = selectedFileIds.map(sid => {
          const f = currentFolders.find(cf => cf.id === sid);
          return f ? -sid : sid;
        });
      }
    } else if (fileRow) {
      // 点击了列表视图的行
      const id = parseInt(fileRow.getAttribute('data-id') || '0');
      if (selectedFileIds.includes(id)) {
        mouseDownInfoRef.current = { x: e.clientX, y: e.clientY, target: fileRow, isGridItem: false, isFileRow: true };
        customDragIdsRef.current = selectedFileIds.map(sid => {
          const f = currentFolders.find(cf => cf.id === sid);
          return f ? -sid : sid;
        });
      }
    } else {
      // 点击了空白区域，准备选择框
      const rect = contentRef.current?.getBoundingClientRect();
      if (!rect || !contentRef.current) return;
      const startX = e.clientX - rect.left + contentRef.current.scrollLeft;
      const startY = e.clientY - rect.top + contentRef.current.scrollTop;
      selectStartRef.current = { x: startX, y: startY };
      mouseDownInfoRef.current = { x: e.clientX, y: e.clientY, target, isGridItem: false, isFileRow: false };
      setIsSelecting(true);
      setSelectionRect({ x: startX, y: startY, w: 0, h: 0 });
    }
  };

  const handleMouseMove = (e: React.MouseEvent) => {
    const info = mouseDownInfoRef.current;
    if (!info) return;

    // 如果还没确定是拖拽还是选择框
    if (!isCustomDragging && !isSelecting) {
      const dist = Math.sqrt((e.clientX - info.x) ** 2 + (e.clientY - info.y) ** 2);
      if (dist < 5) return; // 移动距离太小，忽略

      if (info.isGridItem || info.isFileRow) {
        // 开始拖拽已选中的项目
        setIsCustomDragging(true);
        setCustomDragMouse({ x: e.clientX, y: e.clientY });
        return;
      } else {
        // 开始选择框
        const rect = contentRef.current?.getBoundingClientRect();
        if (!rect || !contentRef.current) return;
        const startX = info.x - rect.left + contentRef.current.scrollLeft;
        const startY = info.y - rect.top + contentRef.current.scrollTop;
        selectStartRef.current = { x: startX, y: startY };
        setIsSelecting(true);
        setSelectionRect({ x: startX, y: startY, w: 0, h: 0 });
      }
    }

    // 处理自定义拖拽
    if (isCustomDragging) {
      setCustomDragMouse({ x: e.clientX, y: e.clientY });
      // 检测鼠标是否悬停在侧边栏的文件夹上
      const treeNodes = document.querySelectorAll('.ant-tree-treenode');
      let foundFolderId: number | null = null;
      treeNodes.forEach(node => {
        const rect = node.getBoundingClientRect();
        if (e.clientX >= rect.left && e.clientX <= rect.right &&
            e.clientY >= rect.top && e.clientY <= rect.bottom) {
          const titleEl = node.querySelector('.ant-tree-title');
          if (titleEl) {
            const name = titleEl.textContent || '';
            const folder = folders.find(f => f.name === name);
            if (folder) foundFolderId = folder.folder_id;
          }
        }
      });
      setSidebarHoverFolderId(foundFolderId);
      return;
    }

    // 处理选择框
    if (isSelecting && contentRef.current && selectStartRef.current) {
      const rect = contentRef.current.getBoundingClientRect();
      const startX = selectStartRef.current.x;
      const startY = selectStartRef.current.y;
      const curX = e.clientX - rect.left + contentRef.current.scrollLeft;
      const curY = e.clientY - rect.top + contentRef.current.scrollTop;

      const x = Math.min(startX, curX);
      const y = Math.min(startY, curY);
      const w = Math.abs(curX - startX);
      const h = Math.abs(curY - startY);

      setSelectionRect({ x, y, w, h });

      // 检测哪些项目在选择框内
      const items = contentRef.current.querySelectorAll('.grid-item, .file-row');
      const selectedIds: number[] = [];
      items.forEach((item) => {
        const itemRect = item.getBoundingClientRect();
        const itemLeft = itemRect.left - rect.left + contentRef.current!.scrollLeft;
        const itemTop = itemRect.top - rect.top + contentRef.current!.scrollTop;
        const itemRight = itemLeft + itemRect.width;
        const itemBottom = itemTop + itemRect.height;
        if (itemLeft < x + w && itemRight > x && itemTop < y + h && itemBottom > y) {
          const id = parseInt(item.getAttribute('data-id') || '0');
          const type = item.getAttribute('data-type');
          if (type === 'folder') selectedIds.push(-id);
          else selectedIds.push(id);
        }
      });
      setSelectedFileIds(selectedIds.map(id => Math.abs(id)));
    }
  };

  const handleMouseUp = () => {
    // 完成自定义拖拽
    if (isCustomDragging) {
      if (sidebarHoverFolderId !== null) {
        const ids = customDragIdsRef.current;
        if (ids.length > 0) {
          invoke<BatchResult>('batch_move', { ids, targetFolderId: sidebarHoverFolderId })
            .then(result => {
              if (result.fail_count > 0) {
                message.warning(`移动完成：成功 ${result.success_count} 个，失败 ${result.fail_count} 个`);
              } else {
                message.success(`成功移动 ${result.success_count} 个项目`);
              }
              loadFolderContents(currentFolderId);
              loadFolders();
            })
            .catch(err => message.error('移动失败: ' + err));
        }
      }
      setIsCustomDragging(false);
      setCustomDragMouse(null);
      setSidebarHoverFolderId(null);
      customDragIdsRef.current = [];
      mouseDownInfoRef.current = null;
      return;
    }

    // 完成选择框
    if (isSelecting) {
      setIsSelecting(false);
      setSelectionRect(null);
      selectStartRef.current = null;
      justFinishedSelectionRef.current = true;
      setTimeout(() => {
        justFinishedSelectionRef.current = false;
      }, 100);
    }
    mouseDownInfoRef.current = null;
  };

  const getContextMenuItems = (record: any): MenuProps['items'] => {
    const isFolder = record.category === 'folder' || record.isFolder;
    
    if (isFolder) {
      // 文件夹的右键菜单
      return [
        {
          key: 'open',
          label: '打开文件夹',
          onClick: () => enterFolder(record.file_id),
        },
        { type: 'divider' },
        {
          key: 'rename',
          label: '重命名',
          icon: <EditOutlined />,
          onClick: () => {
            setInlineRenameId(-record.file_id);
            setInlineRenameValue(record.name);
          },
        },
        {
          key: 'delete',
          label: '删除',
          icon: <DeleteOutlined />,
          danger: true,
          onClick: () => {
            Modal.confirm({
              title: '确认删除',
              content: `确定要删除文件夹 "${record.name}" 及其所有内容吗？`,
              onOk: () => handleDelete([record.file_id]),
            });
          },
        },
        { type: 'divider' },
        {
          key: 'export',
          label: '导出到...',
          icon: <ExportOutlined />,
          onClick: () => {
            setSelectedFileIds([record.file_id]);
            handleExport();
          },
        },
      ];
    }
    
    // 文件的右键菜单
    return [
      {
        key: 'open',
        label: '打开',
        onClick: () => handleOpenFile(record.file_id),
      },
      { type: 'divider' },
      {
        key: 'rename',
        label: '重命名',
        icon: <EditOutlined />,
        onClick: () => {
          setRenameFileId(record.file_id);
          setRenameValue(record.name);
          setRenameModalOpen(true);
        },
      },
      {
        key: 'delete',
        label: '删除',
        icon: <DeleteOutlined />,
        danger: true,
        onClick: () => {
          Modal.confirm({
            title: '确认删除',
            content: `确定要删除 "${record.name}" 吗？`,
            onOk: () => handleDelete([record.file_id]),
          });
        },
      },
      { type: 'divider' },
      {
        key: 'export',
        label: '导出到...',
        icon: <ExportOutlined />,
        onClick: () => {
          setSelectedFileIds([record.file_id]);
          handleExport();
        },
      },
    ];
  };

  return (
    <Layout style={{ height: '100vh' }}>
      {/* Toolbar */}
      <Layout.Header style={{ background: '#fff', padding: '0 16px', borderBottom: '1px solid #f0f0f0', display: 'flex', alignItems: 'center', justifyContent: 'space-between', height: 48 }}>
        <Space>
          <Tooltip title="导入文件">
            <Button icon={<ImportOutlined />} onClick={handleImportFiles}>导入文件</Button>
          </Tooltip>
          <Tooltip title="导入文件夹">
            <Button icon={<FolderAddOutlined />} onClick={handleImportFolders}>导入文件夹</Button>
          </Tooltip>
          <Tooltip title="导出选中">
            <Button icon={<ExportOutlined />} onClick={handleExport} disabled={selectedFileIds.length === 0}>导出</Button>
          </Tooltip>
          <Tooltip title="删除选中">
            <Button icon={<DeleteOutlined />} onClick={() => handleDelete()} danger disabled={selectedFileIds.length === 0}>删除</Button>
          </Tooltip>
          <Tooltip title="新建文件夹">
            <Button icon={<FolderAddOutlined />} onClick={() => setNewFolderModalOpen(true)}>新建文件夹</Button>
          </Tooltip>
          <Tooltip title="刷新">
            <Button icon={<ReloadOutlined />} onClick={() => loadFolderContents(currentFolderId)} />
          </Tooltip>
        </Space>
        <Space>
          <Input.Search
            placeholder="搜索文件名..."
            value={searchKeyword}
            onChange={e => setSearchKeyword(e.target.value)}
            onSearch={handleSearch}
            style={{ width: 250 }}
            allowClear
          />
          <Button
            icon={viewMode === 'list' ? <UnorderedListOutlined /> : <AppstoreOutlined />}
            onClick={() => setViewMode(viewMode === 'list' ? 'grid' : 'list')}
          />
        </Space>
      </Layout.Header>

      <Layout style={{ flex: 1, overflow: 'hidden' }}>
        {/* Sidebar - Directory Tree */}
        <Layout.Sider 
          width={180} 
          style={{ background: '#fff', borderRight: '1px solid #f0f0f0', overflow: 'auto' }}
        >
          <div style={{ padding: '8px 12px', fontWeight: 'bold', borderBottom: '1px solid #f0f0f0' }}>
            <HomeOutlined /> 目录
          </div>
          <Tree
            showIcon
            expandedKeys={expandedKeys}
            onExpand={(keys) => setExpandedKeys(keys)}
            treeData={buildTreeData(folders)}
            selectedKeys={[currentFolderId]}
            onSelect={handleFolderSelect}
            style={{ padding: '8px' }}
          />
        </Layout.Sider>

        {/* Main Content */}
        <Layout.Content style={{ overflow: 'auto', background: '#fff' }}>
          {/* Breadcrumb */}
          <div style={{ padding: '8px 16px', borderBottom: '1px solid #f0f0f0' }}>
            <Breadcrumb>
              {breadcrumb.map((item, idx) => (
                <Breadcrumb.Item
                  key={item.id}
                  onClick={() => {
                    setCurrentFolderId(item.id);
                    setBreadcrumb(breadcrumb.slice(0, idx + 1));
                  }}
                  style={{ cursor: 'pointer' }}
                >
                  {item.name}
                </Breadcrumb.Item>
              ))}
            </Breadcrumb>
          </div>

          {/* File List / Grid */}
          {viewMode === 'list' ? (
            <div
              ref={contentRef}
              tabIndex={0}
              onMouseDown={handleMouseDown}
              onMouseMove={handleMouseMove}
              onMouseUp={handleMouseUp}
              onClick={() => {
                if (justFinishedSelectionRef.current) return;
                setSelectedFileIds([]);
                setInlineRenameId(null);
                setContextMenuRecord(null);
                setContextMenuPos(null);
              }}
              onKeyDown={(e) => {
                if (e.ctrlKey && e.key === 'a') {
                  e.preventDefault();
                  const allIds = items.map(i => i.id);
                  setSelectedFileIds(allIds);
                }
              }}
              style={{ outline: 'none', height: '100%', overflow: 'auto', cursor: 'default', position: 'relative' }}
            >
              <Table
                dataSource={items.map(item => ({
                  key: item.is_folder ? `folder-${item.id}` : `file-${item.id}`,
                  file_id: item.id,
                  name: item.name,
                  size_bytes: item.size_bytes,
                  category: item.is_folder ? 'folder' : item.category,
                  ext: item.ext,
                  thumbnail_path: item.thumbnail_path,
                  updated_at: item.updated_at,
                  isFolder: item.is_folder,
                }))}
                columns={[
                  {
                    title: '',
                    key: 'spacer',
                    width: 8,
                    render: () => <span style={{ width: 8, display: 'inline-block' }} />,
                  },
                  {
                    title: '文件名',
                    dataIndex: 'name',
                    key: 'name',
                    render: (name: string, record: any) => (
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
                        {record.isFolder ? (
                          <FolderOutlined style={{ color: '#faad14', fontSize: 16 }} />
                        ) : record.thumbnail_path ? (
                          <img
                            src={convertFileSrc(record.thumbnail_path)}
                            alt={name}
                            style={{ width: 28, height: 28, objectFit: 'cover', borderRadius: 3, flexShrink: 0 }}
                          />
                        ) : (
                          <span style={{ fontSize: 16, display: 'inline-flex' }}>
                            {getFileIcon(record.category, record.ext)}
                          </span>
                        )}
                        <span>{name}</span>
                      </span>
                    ),
                  },
                  {
                    title: '大小',
                    dataIndex: 'size_bytes',
                    key: 'size',
                    width: 100,
                    render: (size: number, record: any) => record.isFolder ? '-' : formatFileSize(size),
                  },
                  {
                    title: '类型',
                    dataIndex: 'category',
                    key: 'category',
                    width: 80,
                    render: (cat: string, record: any) => {
                      if (record.isFolder) return '文件夹';
                      switch (cat) {
                        case 'image': return '图片';
                        case 'video_short': return '短视频';
                        case 'video_long': return '长视频';
                        case 'doc': return '文档';
                        default: return cat || '-';
                      }
                    },
                  },
                  {
                    title: '修改时间',
                    dataIndex: 'updated_at',
                    key: 'updated_at',
                    width: 180,
                    render: (t: number) => formatTime(t),
                  },
                ]}
                pagination={false}
                size="small"
                rowClassName={(record: any) => selectedFileIds.includes(record.file_id) ? 'file-row file-row-selected' : 'file-row'}
                onRow={(record: any) => ({
                  'data-id': record.file_id,
                  'data-type': record.isFolder ? 'folder' : 'file',
                  onMouseDown: (e: React.MouseEvent) => {
                    // 如果点击的是已选中的行，准备拖拽
                    if (selectedFileIds.includes(record.file_id)) {
                      const row = (e.target as HTMLElement).closest('.file-row') as HTMLElement;
                      if (row) {
                        mouseDownInfoRef.current = { x: e.clientX, y: e.clientY, target: row, isGridItem: false, isFileRow: true };
                        customDragIdsRef.current = selectedFileIds.map(sid => {
                          const f = currentFolders.find(cf => cf.id === sid);
                          return f ? -sid : sid;
                        });
                      }
                    }
                  },
                  onClick: (e) => {
                    e.stopPropagation();
                    if (e.ctrlKey) {
                      setSelectedFileIds(prev =>
                        prev.includes(record.file_id)
                          ? prev.filter(id => id !== record.file_id)
                          : [...prev, record.file_id]
                      );
                    } else {
                      setSelectedFileIds([record.file_id]);
                    }
                  },
                  onDoubleClick: () => {
                    if (record.isFolder) {
                      enterFolder(record.file_id);
                    } else {
                      handleOpenFile(record.file_id);
                    }
                  },
                  onContextMenu: (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    // 设置右键菜单
                    setContextMenuRecord(record);
                    setContextMenuPos({ x: e.clientX, y: e.clientY });
                  },
                })}
                scroll={{ y: 'calc(100vh - 180px)' }}
              />
              {/* 列表视图选择框 */}
              {selectionRect && selectionRect.w > 2 && selectionRect.h > 2 && (
                <div
                  style={{
                    position: 'absolute',
                    left: selectionRect.x,
                    top: selectionRect.y,
                    width: selectionRect.w,
                    height: selectionRect.h,
                    border: '1px solid #1890ff',
                    background: 'rgba(24, 144, 255, 0.1)',
                    pointerEvents: 'none',
                    zIndex: 1000,
                  }}
                />
              )}
            </div>
          ) : (
            <div
              ref={contentRef}
              tabIndex={0}
              onKeyDown={(e) => {
                // Ctrl+A 全选
                if (e.ctrlKey && e.key === 'a') {
                  e.preventDefault();
                  const allIds = items.map(i => i.id);
                  setSelectedFileIds(allIds);
                }
              }}
              onMouseDown={handleMouseDown}
              onMouseMove={handleMouseMove}
              onMouseUp={handleMouseUp}
              style={{ padding: 16, display: 'flex', flexWrap: 'wrap', gap: 12, overflow: 'auto', outline: 'none', cursor: 'default', position: 'relative', minHeight: 'calc(100vh - 180px)', alignContent: 'flex-start' }}
              onClick={() => {
                // 如果刚完成框选，不清空选中状态
                if (justFinishedSelectionRef.current) return;
                setSelectedFileIds([]);
                setInlineRenameId(null);
                setContextMenuRecord(null);
                setContextMenuPos(null);
              }}
            >
              {items.map(item => (
                <Dropdown
                  key={item.is_folder ? `folder-${item.id}` : `file-${item.id}`}
                  menu={{ items: getContextMenuItems({
                    file_id: item.id,
                    name: item.name,
                    category: item.is_folder ? 'folder' : item.category,
                    size_bytes: item.size_bytes,
                    ext: item.ext,
                    mime_type: null,
                    duration_sec: null,
                    width: null,
                    height: null,
                    thumbnail_path: item.thumbnail_path,
                    created_at: item.updated_at,
                    updated_at: item.updated_at,
                  } as FileInfo) }}
                  trigger={['contextMenu']}
                >
                <div
                  className={`grid-item ${selectedFileIds.includes(item.id) ? 'grid-item-selected' : ''}`}
                  data-id={item.id}
                  data-type={item.is_folder ? 'folder' : 'file'}
                  style={{
                    width: 120,
                    padding: 8,
                    textAlign: 'center',
                    borderRadius: 4,
                    border: selectedFileIds.includes(item.id) ? '2px solid #1890ff' : '1px solid #f0f0f0',
                  }}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (e.ctrlKey) {
                      setSelectedFileIds(prev =>
                        prev.includes(item.id)
                          ? prev.filter(id => id !== item.id)
                          : [...prev, item.id]
                      );
                    } else {
                      setSelectedFileIds([item.id]);
                      handleClickForRename(item.is_folder ? -item.id : item.id, item.name);
                    }
                  }}
                  onDoubleClick={() => {
                    setInlineRenameId(null);
                    if (item.is_folder) {
                      enterFolder(item.id);
                    } else {
                      handleOpenFile(item.id);
                    }
                  }}
                >
                  {item.is_folder ? (
                    <>
                      <div style={{ fontSize: 40, marginBottom: 4 }}>
                        <FolderOutlined style={{ color: '#faad14' }} />
                      </div>
                      <div style={{ fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {inlineRenameId === -item.id ? (
                          <Input
                            size="small"
                            value={inlineRenameValue}
                            onChange={e => setInlineRenameValue(e.target.value)}
                            onBlur={handleInlineRenameSubmit}
                            onPressEnter={handleInlineRenameSubmit}
                            onKeyDown={e => { if (e.key === 'Escape') setInlineRenameId(null); }}
                            autoFocus
                            onClick={e => e.stopPropagation()}
                            style={{ fontSize: 12, padding: '0 4px' }}
                          />
                        ) : item.name}
                      </div>
                      <div style={{ fontSize: 11, color: '#999' }}>文件夹</div>
                    </>
                  ) : (
                    <>
                      <div style={{ width: 80, height: 80, marginBottom: 4, display: 'flex', alignItems: 'center', justifyContent: 'center', overflow: 'hidden' }}>
                        {item.thumbnail_path ? (
                          <img
                            src={convertFileSrc(item.thumbnail_path)}
                            alt={item.name}
                            style={{ width: '100%', height: '100%', objectFit: 'cover' }}
                          />
                        ) : (
                          <div style={{ fontSize: 40, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                            {getFileIcon(item.category, item.ext)}
                          </div>
                        )}
                      </div>
                      <div style={{ fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {inlineRenameId === item.id ? (
                          <Input
                            size="small"
                            value={inlineRenameValue}
                            onChange={e => setInlineRenameValue(e.target.value)}
                            onBlur={handleInlineRenameSubmit}
                            onPressEnter={handleInlineRenameSubmit}
                            onKeyDown={e => { if (e.key === 'Escape') setInlineRenameId(null); }}
                            autoFocus
                            onClick={e => e.stopPropagation()}
                            style={{ fontSize: 12, padding: '0 4px' }}
                          />
                        ) : item.name}
                      </div>
                      <div style={{ fontSize: 11, color: '#999' }}>{formatFileSize(item.size_bytes)}</div>
                    </>
                  )}
                </div>
                </Dropdown>
              ))}
              {/* 拖拽选择框 */}
              {selectionRect && selectionRect.w > 2 && selectionRect.h > 2 && (
                <div
                  style={{
                    position: 'absolute',
                    left: selectionRect.x,
                    top: selectionRect.y,
                    width: selectionRect.w,
                    height: selectionRect.h,
                    border: '1px solid #1890ff',
                    background: 'rgba(24, 144, 255, 0.1)',
                    pointerEvents: 'none',
                    zIndex: 1000,
                  }}
                />
              )}
            </div>
          )}
        </Layout.Content>
      </Layout>

      {/* 自定义拖拽覆盖层 */}
      {isCustomDragging && customDragMouse && (
        <>
          <div
            style={{
              position: 'fixed',
              left: customDragMouse.x + 12,
              top: customDragMouse.y + 12,
              background: '#fff',
              border: '1px solid #1890ff',
              borderRadius: 4,
              padding: '4px 8px',
              fontSize: 12,
              boxShadow: '0 2px 8px rgba(0,0,0,0.15)',
              pointerEvents: 'none',
              zIndex: 10000,
            }}
          >
            移动 {customDragIdsRef.current.length} 个项目
          </div>
          {sidebarHoverFolderId !== null && (
            <div
              style={{
                position: 'fixed',
                left: customDragMouse.x + 12,
                top: customDragMouse.y + 36,
                background: '#e6f7ff',
                border: '1px solid #1890ff',
                borderRadius: 4,
                padding: '2px 6px',
                fontSize: 11,
                pointerEvents: 'none',
                zIndex: 10000,
              }}
            >
              释放以移动到: {folders.find(f => f.folder_id === sidebarHoverFolderId)?.name}
            </div>
          )}
        </>
      )}

      {/* Status Bar */}
      <Layout.Footer style={{ height: 28, padding: '0 16px', background: '#fafafa', borderTop: '1px solid #f0f0f0', display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, color: '#666', lineHeight: '28px' }}>
        <span>{files.length} 个文件{currentFolders.length > 0 ? `, ${currentFolders.length} 个文件夹` : ''}</span>
        <span>
          {selectedFileIds.length > 0 && `已选择 ${selectedFileIds.length} 个 | `}
          总大小: {formatFileSize(files.reduce((sum, f) => sum + f.size_bytes, 0))}
        </span>
      </Layout.Footer>

      {/* Rename Modal */}
      <Modal
        title="重命名"
        open={renameModalOpen}
        onOk={handleRename}
        onCancel={() => setRenameModalOpen(false)}
      >
        <Input
          value={renameValue}
          onChange={e => setRenameValue(e.target.value)}
          onPressEnter={handleRename}
        />
      </Modal>

      {/* New Folder Modal */}
      <Modal
        title="新建文件夹"
        open={newFolderModalOpen}
        onOk={handleCreateFolder}
        onCancel={() => setNewFolderModalOpen(false)}
      >
        <Input
          placeholder="文件夹名称"
          value={newFolderName}
          onChange={e => setNewFolderName(e.target.value)}
          onPressEnter={handleCreateFolder}
        />
      </Modal>

      {/* 全局右键菜单（用于列表视图） */}
      {contextMenuRecord && contextMenuPos && (
        <>
          {/* 全屏遮罩层，用于捕获点击事件以关闭菜单 */}
          <div
            style={{
              position: 'fixed',
              top: 0,
              left: 0,
              right: 0,
              bottom: 0,
              zIndex: 9999,
            }}
            onClick={() => {
              setContextMenuRecord(null);
              setContextMenuPos(null);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              setContextMenuRecord(null);
              setContextMenuPos(null);
            }}
          />
          {/* 右键菜单 */}
          <div
            style={{
              position: 'fixed',
              left: contextMenuPos.x,
              top: contextMenuPos.y,
              zIndex: 10000,
              background: '#fff',
              border: '1px solid #d9d9d9',
              borderRadius: 4,
              boxShadow: '0 2px 8px rgba(0,0,0,0.15)',
              padding: '4px 0',
            }}
          >
            <Menu
              items={getContextMenuItems(contextMenuRecord)}
              onClick={() => {
                setContextMenuRecord(null);
                setContextMenuPos(null);
              }}
              style={{ border: 'none' }}
            />
          </div>
        </>
      )}
    </Layout>
  );
}

export default App;

'use client';

import React, {
    ComponentType,
    useCallback,
    useEffect,
    useImperativeHandle,
    useMemo,
    useRef,
    useState,
} from 'react';
import * as PIXI from 'pixi.js';
import { ElementRect, ImageBuffer } from '@/commands';
import styles from './index.module.css';
import { last } from 'es-toolkit';
import { MonitorInfo } from '@/commands/core';
import { releaseDrawPage } from '@/functions/screenshot';

export type BaseLayerContextType = {
    /** 调整画布大小 */
    resizeCanvas: (width: number, height: number) => void;
    /** 添加一个子元素到最上层的容器 */
    addChildToTopContainer: (children: PIXI.Container<PIXI.ContainerChild>) => void;
    /** 创建一个新的画布容器 */
    createNewCanvasContainer: () => PIXI.Container | undefined;
    /** 清空画布 */
    clearCanvas: () => void;
    /** 是否启用 */
    isEnable: boolean;
    /**
     * 改变光标样式
     */
    changeCursor: (cursor: Required<React.CSSProperties>['cursor']) => string;
    /** 画布容器元素 */
    layerContainerElementRef: React.RefObject<HTMLDivElement | null>;
    /** 获取画布上下文 */
    getCanvasApp: () => PIXI.Application | undefined;
};

export const BaseLayerContext = React.createContext<BaseLayerContextType>({
    resizeCanvas: () => {},
    addChildToTopContainer: () => {},
    createNewCanvasContainer: () => undefined,
    clearCanvas: () => {},
    isEnable: false,
    /**
     * 改变光标样式
     */
    changeCursor: () => 'auto',
    /** 画布容器元素 */
    layerContainerElementRef: { current: null },
    /** 获取画布上下文 */
    getCanvasApp: () => undefined,
});

export type BaseLayerEventActionType = {
    /**
     * 初始化画布
     */
    initCanvas: (antialias: boolean) => Promise<void>;
    /**
     * 执行截图
     */
    onExecuteScreenshot: () => Promise<void>;
    /**
     * 画布准备就绪
     */
    onCanvasReady: (app: PIXI.Application) => void;
    /**
     * 截图准备
     */
    onCaptureReady: (
        texture: PIXI.Texture,
        imageBuffer: ImageBuffer,
        monitorInfo: MonitorInfo,
    ) => Promise<void>;
    /**
     * 显示器信息准备
     */
    onMonitorInfoReady: (monitorInfo: MonitorInfo) => Promise<void>;
    /**
     * 截图加载完成
     */
    onCaptureLoad: (
        texture: PIXI.Texture,
        imageBuffer: ImageBuffer,
        monitorInfo: MonitorInfo,
    ) => Promise<void>;
    /**
     * 截图完成
     */
    onCaptureFinish: () => Promise<void>;
    /**
     * 获取画布上下文
     */
    getCanvasApp: () => PIXI.Application | undefined | null;
    /**
     * 获取画布容器元素
     */
    getLayerContainerElement: () => HTMLDivElement | null;
    /**
     * 添加一个子元素到最上层的容器
     */
    addChildToTopContainer: (children: PIXI.Container<PIXI.ContainerChild>) => void;
    /**
     * 添加一个子元素到指定的容器
     */
    addChildToContainer: (
        container: PIXI.Container<PIXI.ContainerChild>,
        children: PIXI.Container<PIXI.ContainerChild>,
    ) => void;
    /**
     * 创建一个新的画布容器
     */
    createNewCanvasContainer: () => PIXI.Container | undefined;
    /**
     * 获取图片数据
     */
    getImageData: (selectRect: ElementRect) => Promise<ImageData | undefined>;
    /**
     * 获取画布
     */
    getCanvas: () => PIXI.ICanvas | undefined;
};

export type BaseLayerActionType = {
    /**
     * 设置是否启用
     */
    setEnable: (enable: boolean) => void;
    /**
     * 改变光标样式
     */
    changeCursor: (cursor: Required<React.CSSProperties>['cursor']) => string;
} & BaseLayerEventActionType;

export const defaultBaseLayerActions: BaseLayerActionType = {
    initCanvas: () => Promise.resolve(),
    onCanvasReady: () => {},
    onCaptureReady: () => Promise.resolve(),
    onCaptureLoad: () => Promise.resolve(),
    onCaptureFinish: () => Promise.resolve(),
    onMonitorInfoReady: () => Promise.resolve(),
    setEnable: () => {},
    onExecuteScreenshot: () => Promise.resolve(),
    getCanvasApp: () => null,
    getLayerContainerElement: () => null,
    addChildToTopContainer: () => {},
    changeCursor: () => 'auto',
    createNewCanvasContainer: () => undefined,
    addChildToContainer: () => {},
    getImageData: () => Promise.resolve(undefined),
    getCanvas: () => undefined,
};

type BaseLayerCoreActionType = {
    /**
     * 初始化画布
     */
    initCanvas: (antialias: boolean) => Promise<void>;
    resizeCanvas: (width: number, height: number) => void;
    clearCanvas: () => Promise<void>;
    getCanvasApp: () => PIXI.Application | undefined;
    getLayerContainerElement: () => HTMLDivElement | null;
    addChildToTopContainer: (children: PIXI.Container<PIXI.ContainerChild>) => void;
    addChildToContainer: (
        container: PIXI.Container<PIXI.ContainerChild>,
        children: PIXI.Container<PIXI.ContainerChild>,
    ) => void;
    changeCursor: (cursor: Required<React.CSSProperties>['cursor']) => string;
    getImageData: (selectRect: ElementRect) => Promise<ImageData | undefined>;
    getCanvas: () => PIXI.ICanvas | undefined;
};

export type BaseLayerProps = {
    children: React.ReactNode;
    zIndex: number;
    enable: boolean;
    onCanvasReadyAction: (app: PIXI.Application) => void;
};

export const BaseLayerCore: React.FC<
    BaseLayerProps & { actionRef?: React.RefObject<BaseLayerCoreActionType | undefined> }
> = ({ zIndex, actionRef, enable, children, onCanvasReadyAction }) => {
    const layerContainerElementRef = useRef<HTMLDivElement>(null);
    const canvasAppRef = useRef<PIXI.Application | undefined>(undefined);
    const canvasContainerListRef = useRef<PIXI.Container[]>([]);
    const canvasContainerChildCountRef = useRef<number>(0);

    /** 创建一个新的画布容器 */
    const createNewCanvasContainer = useCallback((): PIXI.Container | undefined => {
        const canvasApp = canvasAppRef.current;
        if (!canvasApp) {
            return;
        }

        const container = new PIXI.Container();
        container.zIndex = canvasContainerListRef.current.length + 1;
        container.sortableChildren = true;
        container.x = 0;
        container.y = 0;
        canvasApp.stage.addChild(container);
        canvasContainerListRef.current.push(container);

        return container;
    }, []);

    const disposeCanvas = useCallback(() => {
        const canvasApp = canvasAppRef.current;
        if (!canvasApp) {
            return;
        }
        layerContainerElementRef.current?.removeChild(canvasApp.canvas);
        canvasApp.destroy();
    }, []);

    /** 初始化画布 */
    const initedRef = useRef(false);
    const initCanvas = useCallback<BaseLayerCoreActionType['initCanvas']>(
        async (antialias: boolean) => {
            if (initedRef.current) {
                return;
            }

            initedRef.current = true;

            disposeCanvas();

            const canvasApp = new PIXI.Application();
            await canvasApp.init({
                backgroundAlpha: 0,
                eventFeatures: {
                    move: false,
                    globalMove: false,
                    click: false,
                    wheel: false,
                },
                autoStart: false,
                antialias,
                preserveDrawingBuffer: true,
            });
            canvasApp.ticker.maxFPS = 60;
            canvasApp.ticker.minFPS = 0;
            canvasAppRef.current = canvasApp;
            layerContainerElementRef.current?.appendChild(canvasApp.canvas);

            onCanvasReadyAction(canvasApp);

            releaseDrawPage();
        },
        [disposeCanvas, onCanvasReadyAction],
    );

    /** 调整画布大小 */
    const resizeCanvas = useCallback(
        (width: number, height: number) => {
            const canvasApp = canvasAppRef.current;
            if (!canvasApp) {
                return;
            }

            // 创建根画布容器
            createNewCanvasContainer();

            canvasApp.renderer.resize(width, height);
        },
        [createNewCanvasContainer],
    );

    const getTopContainer = useCallback(() => {
        return last(canvasContainerListRef.current);
    }, []);

    const addChildToTopContainer = useCallback(
        (children: PIXI.Container<PIXI.ContainerChild>) => {
            const topContainer = getTopContainer();
            if (!topContainer) {
                throw new Error('Top container not found');
            }
            children.zIndex = canvasContainerChildCountRef.current + 1;
            topContainer.addChild(children);
            canvasContainerChildCountRef.current++;
        },
        [getTopContainer],
    );

    const addChildToContainer = useCallback(
        (
            container: PIXI.Container<PIXI.ContainerChild>,
            children: PIXI.Container<PIXI.ContainerChild>,
        ) => {
            children.zIndex = canvasContainerChildCountRef.current + 1;
            container.addChild(children);
            canvasContainerChildCountRef.current++;
        },
        [],
    );

    const clearCanvas = useCallback<BaseLayerCoreActionType['clearCanvas']>(async () => {
        const canvasApp = canvasAppRef.current;
        if (!canvasApp) {
            return;
        }
        while (canvasApp.stage.children[0]) {
            canvasApp.stage.removeChild(canvasApp.stage.children[0]);
        }
        canvasContainerListRef.current = [];
        canvasContainerChildCountRef.current = 0;
        canvasApp.render();
    }, []);

    const changeCursor = useCallback<BaseLayerContextType['changeCursor']>((cursor) => {
        const previousCursor = layerContainerElementRef.current!.style.cursor;
        layerContainerElementRef.current!.style.cursor = cursor;
        return previousCursor;
    }, []);

    const getCanvasApp = useCallback<BaseLayerContextType['getCanvasApp']>(() => {
        return canvasAppRef.current;
    }, []);

    const getLayerContainerElement = useCallback<
        BaseLayerCoreActionType['getLayerContainerElement']
    >(() => layerContainerElementRef.current, []);

    const getImageData = useCallback<BaseLayerActionType['getImageData']>(
        async (selectRect: ElementRect) => {
            const canvasApp = canvasAppRef.current;
            if (!canvasApp) {
                return;
            }
            return canvasApp.renderer.extract
                .canvas(canvasApp.stage)
                .getContext('2d')
                ?.getImageData(
                    selectRect.min_x,
                    selectRect.min_y,
                    selectRect.max_x - selectRect.min_x,
                    selectRect.max_y - selectRect.min_y,
                    {
                        colorSpace: 'srgb',
                    },
                );
        },
        [],
    );

    const getCanvas = useCallback<BaseLayerActionType['getCanvas']>(() => {
        const canvasApp = canvasAppRef.current;
        if (!canvasApp) {
            return;
        }

        return canvasApp.renderer.extract.canvas(canvasApp.stage);
    }, []);

    useEffect(() => {
        return () => {
            disposeCanvas();
        };
    }, [initCanvas, disposeCanvas]);

    useImperativeHandle(
        actionRef,
        () => ({
            resizeCanvas,
            clearCanvas,
            getCanvasApp,
            getLayerContainerElement,
            addChildToTopContainer,
            addChildToContainer,
            changeCursor,
            createNewCanvasContainer,
            getImageData,
            getCanvas,
            initCanvas,
        }),
        [
            resizeCanvas,
            clearCanvas,
            getCanvasApp,
            getLayerContainerElement,
            addChildToTopContainer,
            addChildToContainer,
            changeCursor,
            createNewCanvasContainer,
            getImageData,
            getCanvas,
            initCanvas,
        ],
    );

    useEffect(() => {
        if (enable) {
            layerContainerElementRef.current!.style.pointerEvents = 'auto';
        } else {
            layerContainerElementRef.current!.style.pointerEvents = 'none';
        }
    }, [enable]);

    const baseLayerContextValue = useMemo(() => {
        return {
            resizeCanvas,
            addChildToTopContainer,
            clearCanvas,
            createNewCanvasContainer,
            isEnable: enable,
            changeCursor,
            layerContainerElementRef,
            getCanvasApp,
        };
    }, [
        resizeCanvas,
        addChildToTopContainer,
        clearCanvas,
        createNewCanvasContainer,
        enable,
        changeCursor,
        getCanvasApp,
    ]);

    return (
        <BaseLayerContext.Provider value={baseLayerContextValue}>
            <div className={styles.baseLayer} ref={layerContainerElementRef} style={{ zIndex }} />
            {children}
        </BaseLayerContext.Provider>
    );
};

/**
 * BaseLayer 高阶组件
 * @param WrappedComponent 需要包装的基础组件
 */
export function withBaseLayer<
    ActionType extends BaseLayerActionType,
    Props extends { actionRef: React.RefObject<ActionType | undefined> },
>(WrappedComponent: ComponentType<Props>, zIndex: number) {
    return React.memo(function BaseLayer(props: Props) {
        const { actionRef } = props;
        const layerActionRef = useRef<ActionType | undefined>(undefined);
        const baseLayerCoreActionRef = useRef<BaseLayerCoreActionType | undefined>(undefined);

        const [layerEnable, setLayerEnable] = useState(false);

        const initCanvas = useCallback<BaseLayerActionType['initCanvas']>(
            async (antialias: boolean) => {
                await baseLayerCoreActionRef.current?.initCanvas(antialias);
            },
            [],
        );
        const onCaptureReady = useCallback(
            async (...args: Parameters<BaseLayerActionType['onCaptureReady']>) => {
                await layerActionRef.current?.onCaptureReady(...args);
            },
            [],
        );
        const onMonitorInfoReady = useCallback(
            async (...args: Parameters<BaseLayerActionType['onMonitorInfoReady']>) => {
                const [monitorInfo] = args;

                // 将画布调整为截图大小
                const { monitor_width, monitor_height } = monitorInfo;

                baseLayerCoreActionRef.current?.resizeCanvas(monitor_width, monitor_height);
                await layerActionRef.current?.onMonitorInfoReady(...args);
            },
            [],
        );
        const onCaptureFinish = useCallback(
            async (...args: Parameters<BaseLayerActionType['onCaptureFinish']>) => {
                await Promise.all([
                    baseLayerCoreActionRef.current?.clearCanvas(),
                    layerActionRef.current?.onCaptureFinish(...args),
                ]);
            },
            [],
        );

        const setEnable = useCallback((...args: Parameters<BaseLayerActionType['setEnable']>) => {
            setLayerEnable(...args);
            layerActionRef.current?.setEnable(...args);
        }, []);

        const onCanvasReadyAction = useCallback((app: PIXI.Application) => {
            layerActionRef.current?.onCanvasReady(app);
        }, []);

        const getCanvasApp = useCallback(() => {
            return baseLayerCoreActionRef.current?.getCanvasApp();
        }, []);

        const getLayerContainerElement = useCallback(() => {
            return baseLayerCoreActionRef.current?.getLayerContainerElement();
        }, []);

        const addChildToTopContainer = useCallback(
            (children: PIXI.Container<PIXI.ContainerChild>) => {
                baseLayerCoreActionRef.current?.addChildToTopContainer(children);
            },
            [],
        );

        const addChildToContainer = useCallback(
            (
                container: PIXI.Container<PIXI.ContainerChild>,
                children: PIXI.Container<PIXI.ContainerChild>,
            ) => {
                baseLayerCoreActionRef.current?.addChildToContainer(container, children);
            },
            [],
        );
        const changeCursor = useCallback<BaseLayerActionType['changeCursor']>((cursor) => {
            return baseLayerCoreActionRef.current?.changeCursor(cursor) ?? 'auto';
        }, []);

        const getImageData = useCallback<BaseLayerActionType['getImageData']>(
            async (selectRect: ElementRect) => {
                return baseLayerCoreActionRef.current?.getImageData(selectRect);
            },
            [],
        );

        const getCanvas = useCallback<BaseLayerActionType['getCanvas']>(() => {
            return baseLayerCoreActionRef.current?.getCanvas();
        }, []);

        useImperativeHandle(
            actionRef,
            () => ({
                ...(layerActionRef.current as ActionType),
                onCaptureReady,
                onCaptureFinish,
                onMonitorInfoReady,
                setEnable,
                getCanvasApp,
                getLayerContainerElement,
                addChildToTopContainer,
                addChildToContainer,
                changeCursor,
                getImageData,
                getCanvas,
                initCanvas,
            }),
            [
                onCaptureReady,
                onCaptureFinish,
                onMonitorInfoReady,
                setEnable,
                getCanvasApp,
                getLayerContainerElement,
                addChildToTopContainer,
                addChildToContainer,
                changeCursor,
                getImageData,
                getCanvas,
                initCanvas,
            ],
        );

        return (
            <BaseLayerCore
                zIndex={zIndex}
                actionRef={baseLayerCoreActionRef}
                enable={layerEnable}
                onCanvasReadyAction={onCanvasReadyAction}
            >
                <WrappedComponent {...props} actionRef={layerActionRef} />
            </BaseLayerCore>
        );
    });
}

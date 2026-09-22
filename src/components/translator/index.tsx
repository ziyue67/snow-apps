'use client';

import {
    AppSettingsActionContext,
    AppSettingsData,
    AppSettingsGroup,
    AppSettingsLanguage,
} from '@/app/contextWrap';
import { useAppSettingsLoad } from '@/hooks/useAppSettingsLoad';
import { useStateRef } from '@/hooks/useStateRef';
import {
    getTranslationTypes,
    translate,
    translateTextDeepL,
    translateTextGoogleWeb,
    TranslationDomain,
    TranslationType,
    TranslationTypeOption,
} from '@/services/tools/translation';
import { SwapOutlined } from '@ant-design/icons';
import { Button, Col, Flex, Form, InputRef, Row, Select, SelectProps, Spin, theme } from 'antd';
import TextArea from 'antd/es/input/TextArea';
import { debounce } from 'es-toolkit';
import {
    useCallback,
    useContext,
    useEffect,
    useImperativeHandle,
    useMemo,
    useRef,
    useState,
} from 'react';
import { FormattedMessage, useIntl } from 'react-intl';
import {
    ChatApiConfig,
    convertLanguageCodeToDeepLSourceLanguageCode,
    convertLanguageCodeToDeepLTargetLanguageCode,
    convertLanguageCodeToGoogleLanguageCode,
    TranslationApiConfig,
    TranslationApiType,
} from '@/app/settings/functionSettings/extra';
import OpenAI from 'openai';
import { defaultTranslationPrompt, getTranslationPrompt } from '@/app/tools/translation/extra';
import { ModelSelectLabel } from '@/app/tools/chat/components/modelSelectLabel';
import React from 'react';
import { appFetch } from '@/services/tools';

const SelectLabel: React.FC<{
    label: React.ReactNode;
    code: React.ReactNode;
}> = ({ label, code }) => {
    const { token } = theme.useToken();
    return (
        <div className="language-item">
            <div className="language-item-label">{label}</div>
            <div className="language-item-code">{code}</div>

            <style jsx>{`
                .language-item {
                    position: relative;
                }

                .language-item-label {
                    display: inline;
                }

                .language-item-code {
                    display: inline;
                    color: ${token.colorTextDescription};
                    margin-left: ${token.marginXS}px;
                    font-size: 0.7em;
                    position: relative;
                    bottom: 0.15em;
                }
            `}</style>
        </div>
    );
};

type LanguageItem = {
    code: string;
    label: string;
};

const convertLanguageListToOptions = (list: LanguageItem[]): SelectProps['options'] => {
    // 按首字母分组
    const groupedLanguages = list.reduce(
        (acc, lang) => {
            const firstChar = lang.code.charAt(0).toUpperCase();
            if (!acc[firstChar]) {
                acc[firstChar] = [];
            }
            acc[firstChar].push(lang);
            return acc;
        },
        {} as Record<string, LanguageItem[]>,
    );

    // 转换为 Select 选项格式
    return Object.entries(groupedLanguages).map(([key, languages]) => ({
        label: <span>{key}</span>,
        title: key,
        options: languages.map((lang) => ({
            label: <SelectLabel label={lang.label} code={lang.code.toUpperCase()} />,
            title: `${lang.label}(${lang.code.toLowerCase()})`,
            value: lang.code,
        })),
    }));
};

const selectFilterOption: SelectProps['filterOption'] = (input, option) => {
    if (!input || !option?.title) return false;
    const pattern = input.toLowerCase().split('').join('.*');
    const regex = new RegExp(pattern, 'i');
    return regex.test(option.title.toString().toLowerCase());
};

type TranslationServiceConfig =
    | TranslationTypeOption
    | {
          name: string;
          type: string;
          apiConfig: ChatApiConfig;
      }
    | {
          name: string;
          type: TranslationApiType;
          translationApiConfig: TranslationApiConfig;
      };

export type TranslatorActionType = {
    getTranslatedContentRef: () => string;
    setSourceContent: (content: string, ignoreDebounce?: boolean) => void;
    getSourceContentRef: () => InputRef | null;
    stopTranslate: () => void;
    getTranslationType: () => TranslationType | string;
};

const TranslatorCore: React.FC<{
    actionRef: React.RefObject<TranslatorActionType | undefined>;
    onTranslateComplete?: (result: string) => void;
    disableInput?: boolean;
    tryCatchTranslation?: boolean;
}> = ({ actionRef, onTranslateComplete, disableInput, tryCatchTranslation }) => {
    const intl = useIntl();
    const defaultTranslationTypes: TranslationTypeOption[] = useMemo(
        () => [
            {
                type: TranslationType.Youdao,
                name: intl.formatMessage({ id: 'tools.translation.type.youdao' }),
            },
            {
                type: TranslationType.DeepSeek,
                name: intl.formatMessage({ id: 'tools.translation.type.deepseek' }),
            },
        ],
        [intl],
    );
    const getTranslationApiConfigTypeName = useCallback(
        (apiConfigType: TranslationApiType) => {
            switch (apiConfigType) {
                case TranslationApiType.DeepL:
                    return intl.formatMessage({ id: 'tools.translation.type.deepl' });
                case TranslationApiType.GoogleWeb:
                    return intl.formatMessage({ id: 'tools.translation.type.googleWeb' });
                default:
                    return apiConfigType;
            }
        },
        [intl],
    );

    const { token } = theme.useToken();

    const [languageOptions, languageCodeLabelMap] = useMemo(() => {
        const languageCodeLabelMap = new Map<string, string>();
        const languageList = [
            {
                code: 'en',
                label: intl.formatMessage({ id: 'tools.translation.language.english' }),
            },
            {
                code: 'zh-CHS',
                label: intl.formatMessage({ id: 'tools.translation.language.simplifiedChinese' }),
            },
            {
                code: 'zh-CHT',
                label: intl.formatMessage({ id: 'tools.translation.language.traditionalChinese' }),
            },
            {
                code: 'es',
                label: intl.formatMessage({ id: 'tools.translation.language.spanish' }),
            },
            {
                code: 'fr',
                label: intl.formatMessage({ id: 'tools.translation.language.french' }),
            },
            {
                code: 'ar',
                label: intl.formatMessage({ id: 'tools.translation.language.arabic' }),
            },
            {
                code: 'de',
                label: intl.formatMessage({ id: 'tools.translation.language.german' }),
            },
            {
                code: 'it',
                label: intl.formatMessage({ id: 'tools.translation.language.italian' }),
            },
            {
                code: 'ja',
                label: intl.formatMessage({ id: 'tools.translation.language.japanese' }),
            },
            {
                code: 'pt',
                label: intl.formatMessage({ id: 'tools.translation.language.portuguese' }),
            },
            {
                code: 'ru',
                label: intl.formatMessage({ id: 'tools.translation.language.russian' }),
            },
            {
                code: 'tr',
                label: intl.formatMessage({ id: 'tools.translation.language.turkish' }),
            },
        ].sort((a, b) => {
            if (a.code === 'auto') {
                return -1;
            }
            if (b.code === 'auto') {
                return 1;
            }
            return a.code.localeCompare(b.code);
        });

        languageList.forEach((lang) => {
            languageCodeLabelMap.set(lang.code, lang.label);
        });

        return [convertLanguageListToOptions(languageList), languageCodeLabelMap];
    }, [intl]);

    const [sourceLanguage, setSourceLanguage] = useState<string>('auto');
    const [targetLanguage, setTargetLanguage] = useState<string>('zh-CHS');
    const [translationType, setTranslationType, translationTypeRef] = useStateRef<
        TranslationType | string
    >(TranslationType.Youdao);
    const [translationDomain, setTranslationDomain] = useState<TranslationDomain>(
        TranslationDomain.General,
    );

    const [chatApiConfigList, setChatApiConfigList] = useState<ChatApiConfig[] | undefined>(
        undefined,
    );
    const [translationApiConfigList, setTranslationApiConfigList] = useState<
        TranslationApiConfig[] | undefined
    >(undefined);
    const { updateAppSettings } = useContext(AppSettingsActionContext);
    const [chatConfig, setChatConfig] = useState<AppSettingsData[AppSettingsGroup.SystemChat]>();
    const [translationConfig, setTranslationConfig] =
        useState<AppSettingsData[AppSettingsGroup.FunctionTranslation]>();
    useAppSettingsLoad(
        useCallback(
            (settings) => {
                setTranslationDomain(settings[AppSettingsGroup.Cache].translationDomain);
                setTranslationType(settings[AppSettingsGroup.Cache].translationType);
                setChatApiConfigList(settings[AppSettingsGroup.FunctionChat].chatApiConfigList);
                setTranslationApiConfigList(
                    settings[AppSettingsGroup.FunctionTranslation].translationApiConfigList,
                );
                setChatConfig(settings[AppSettingsGroup.SystemChat]);
                setTranslationConfig(settings[AppSettingsGroup.FunctionTranslation]);
            },
            [setTranslationType, setTranslationDomain],
        ),
    );

    const [supportedTranslationTypes, setSupportedTranslationTypes, supportedTranslationTypesRef] =
        useStateRef<TranslationServiceConfig[]>(defaultTranslationTypes);
    const [onlineTranslationTypes, setOnlineTranslationTypes] = useState<TranslationTypeOption[]>(
        [],
    );
    const [
        supportedTranslationTypesLoading,
        setSupportedTranslationTypesLoading,
        supportedTranslationTypesLoadingRef,
    ] = useStateRef(false);
    useEffect(() => {
        if (supportedTranslationTypesLoadingRef.current) {
            return;
        }

        setSupportedTranslationTypesLoading(true);
        getTranslationTypes().then((res) => {
            setSupportedTranslationTypesLoading(false);
            if (res.response?.status !== 200 || res.code !== 0) {
                return;
            }

            setOnlineTranslationTypes(res.data ?? []);
        });
    }, [setSupportedTranslationTypesLoading, supportedTranslationTypesLoadingRef]);

    useEffect(() => {
        setSupportedTranslationTypes([
            ...(chatApiConfigList?.map((item): TranslationServiceConfig => {
                return {
                    type: `${item.api_model}${item.support_thinking ? '_thinking' : ''}`,
                    name: item.model_name,
                    apiConfig: item,
                };
            }) ?? []),
            ...(translationApiConfigList?.map((item): TranslationServiceConfig => {
                return {
                    type: item.api_type,
                    name: getTranslationApiConfigTypeName(item.api_type),
                    translationApiConfig: item,
                };
            }) ?? []),
            {
                type: TranslationApiType.GoogleWeb,
                name: intl.formatMessage({ id: 'tools.translation.type.googleWeb' }),
                translationApiConfig: {
                    api_type: TranslationApiType.GoogleWeb,
                    api_uri: '',
                    api_key: '',
                },
            },
            ...onlineTranslationTypes,
        ]);
    }, [
        chatApiConfigList,
        getTranslationApiConfigTypeName,
        intl,
        onlineTranslationTypes,
        setSupportedTranslationTypes,
        translationApiConfigList,
    ]);

    const sourceContentRef = useRef<InputRef | null>(null);
    const [sourceContent, setSourceContent] = useState<string>('');
    const [translatedContent, setTranslatedContent, translatedContentRef] = useStateRef<string>('');
    const [autoLanguage, setAutoLanguage] = useState<string | undefined>(undefined);

    const ignoreDebounceRef = useRef<boolean>(false);

    const currentRequestSignRef = useRef<number>(0);
    const [loading, setLoading] = useState(false);
    const [startLoading, setStartLoading] = useState<boolean>(false);

    const customTranslation = useCallback(
        async (params: {
            requestSign: number;
            sourceContent: string;
            sourceLanguage: string;
            targetLanguage: string;
            translationType: string;
            translationDomain: TranslationDomain;
        }): Promise<boolean> => {
            const config = supportedTranslationTypesRef.current.find(
                (item) => item.type === params.translationType,
            );
            if (!config || typeof config.type !== 'string') {
                return false;
            }

            if ('translationApiConfig' in config) {
                setStartLoading(true);

                if (config.type === TranslationApiType.DeepL) {
                    let paramsContent: string[] = [params.sourceContent];

                    if (tryCatchTranslation) {
                        try {
                            const jsonContent = JSON.parse(paramsContent[0]);
                            paramsContent = Object.keys(jsonContent)
                                .sort()
                                .map((key) => jsonContent[key]);
                        } catch {}
                    }

                    const result = await translateTextDeepL(
                        config.translationApiConfig.api_uri,
                        config.translationApiConfig.api_key,
                        paramsContent,
                        convertLanguageCodeToDeepLSourceLanguageCode(params.sourceLanguage),
                        convertLanguageCodeToDeepLTargetLanguageCode(params.targetLanguage),
                        config.translationApiConfig.deepl_prefer_quality_optimized ?? false,
                    );

                    if (!result) {
                        setStartLoading(false);
                        return false;
                    }

                    setTranslatedContent(result.translations.map((item) => item.text).join('\n'));
                } else if (config.type === TranslationApiType.GoogleWeb) {
                    const result = await translateTextGoogleWeb(
                        params.sourceContent,
                        convertLanguageCodeToGoogleLanguageCode(params.sourceLanguage),
                        convertLanguageCodeToGoogleLanguageCode(params.targetLanguage),
                    );

                    if (!result || !Array.isArray(result.sentences)) {
                        setStartLoading(false);
                        return false;
                    }

                    setTranslatedContent(result.sentences.map((item) => item.trans).join('\n'));
                }

                setStartLoading(false);

                onTranslateComplete?.(translatedContentRef.current);
                return true;
            }

            if (!('apiConfig' in config)) {
                return false;
            }

            setStartLoading(true);

            const client = new OpenAI({
                apiKey: config.apiConfig.api_key,
                baseURL: config.apiConfig.api_uri,
                dangerouslyAllowBrowser: true,
                fetch: appFetch,
            });

            const stream_response = await client.chat.completions.create({
                model: config.apiConfig.api_model.replace('_thinking', ''),
                messages: [
                    {
                        role: 'system',
                        content: getTranslationPrompt(
                            translationConfig?.chatPrompt ?? defaultTranslationPrompt,
                            sourceLanguage,
                            targetLanguage,
                            translationDomain,
                        ),
                    },
                    {
                        role: 'user',
                        content: params.sourceContent,
                    },
                ],
                max_completion_tokens: chatConfig?.maxTokens ?? 4096,
                temperature: chatConfig?.temperature ?? 1,
                stream: true,
            });

            if (currentRequestSignRef.current !== params.requestSign) {
                return false;
            }

            setTranslatedContent('');
            for await (const event of stream_response) {
                if (currentRequestSignRef.current !== params.requestSign) {
                    return false;
                }

                setLoading(true);
                setStartLoading(false);

                if (event.choices.length > 0 && event.choices[0].delta.content) {
                    setTranslatedContent(
                        `${translatedContentRef.current}${event.choices[0].delta.content}`,
                    );
                }
            }

            setLoading(false);
            onTranslateComplete?.(translatedContentRef.current);

            return true;
        },
        [
            chatConfig?.maxTokens,
            chatConfig?.temperature,
            onTranslateComplete,
            setTranslatedContent,
            sourceLanguage,
            supportedTranslationTypesRef,
            targetLanguage,
            translatedContentRef,
            translationConfig?.chatPrompt,
            translationDomain,
            tryCatchTranslation,
        ],
    );

    const requestTranslate = useCallback(
        async (params: {
            sourceContent: string;
            sourceLanguage: string;
            targetLanguage: string;
            translationType: TranslationType | string;
            translationDomain: TranslationDomain;
        }) => {
            currentRequestSignRef.current++;
            const requestSign = currentRequestSignRef.current;

            if (typeof params.translationType === 'string') {
                const result = await customTranslation({
                    requestSign,
                    ...params,
                    translationType: params.translationType,
                });
                if (result) {
                    return;
                }
            }

            setStartLoading(true);
            await translate(
                {
                    isInvalid: () => currentRequestSignRef.current !== requestSign,
                    onStart: () => {
                        setStartLoading(false);
                        setLoading(true);
                        setTranslatedContent('');
                    },
                    onData: (response) => {
                        const data = response.success();
                        if (!data) {
                            return;
                        }

                        setTranslatedContent(
                            `${translatedContentRef.current}${data.delta_content}`,
                        );
                        if (
                            params.sourceLanguage === 'auto' &&
                            data.from &&
                            languageCodeLabelMap.has(data.from)
                        ) {
                            setAutoLanguage(data.from);
                        }
                    },
                    onComplete: () => {
                        setLoading(false);
                        onTranslateComplete?.(translatedContentRef.current);
                    },
                },
                {
                    content: params.sourceContent,
                    from: params.sourceLanguage,
                    to: params.targetLanguage,
                    domain: params.translationDomain,
                    type: params.translationType as TranslationType, // 如果没找到自定义模型，则报错
                },
            );
        },
        [
            customTranslation,
            languageCodeLabelMap,
            onTranslateComplete,
            setTranslatedContent,
            translatedContentRef,
        ],
    );

    const requestTranslateDebounce = useMemo(
        () => debounce(requestTranslate, 1000),
        [requestTranslate],
    );

    useEffect(() => {
        if (!chatApiConfigList) {
            return;
        }

        if (sourceContent.trim() === '') {
            return;
        }

        if (ignoreDebounceRef.current) {
            ignoreDebounceRef.current = false;
            requestTranslate({
                sourceContent: sourceContent,
                sourceLanguage: sourceLanguage,
                targetLanguage: targetLanguage,
                translationType,
                translationDomain,
            });
        } else {
            requestTranslateDebounce({
                sourceContent: sourceContent,
                sourceLanguage: sourceLanguage,
                targetLanguage: targetLanguage,
                translationType,
                translationDomain,
            });
        }
    }, [
        sourceContent,
        sourceLanguage,
        targetLanguage,
        requestTranslateDebounce,
        requestTranslate,
        translationType,
        translationDomain,
        chatApiConfigList,
    ]);

    useEffect(() => {
        setAutoLanguage(undefined);
    }, [sourceLanguage]);

    useAppSettingsLoad(
        useCallback((appSettings) => {
            const targetLanguage = appSettings[AppSettingsGroup.Cache].targetLanguage;
            if (targetLanguage) {
                setTargetLanguage(targetLanguage);
            } else if (
                appSettings[AppSettingsGroup.Common].language === AppSettingsLanguage.ZHHant
            ) {
                setTargetLanguage('zh-CHT');
            } else if (
                appSettings[AppSettingsGroup.Common].language === AppSettingsLanguage.ZHHans
            ) {
                setTargetLanguage('zh-CHS');
            } else {
                setTargetLanguage('en');
            }
        }, []),
        true,
    );

    useImperativeHandle(
        actionRef,
        useCallback(
            () => ({
                getTranslatedContentRef: () => translatedContentRef.current,
                setSourceContent: (content: string, ignoreDebounce?: boolean) => {
                    setSourceContent(content);
                    ignoreDebounceRef.current = ignoreDebounce ?? false;
                },
                getSourceContentRef: () => sourceContentRef.current,
                stopTranslate: () => {
                    currentRequestSignRef.current++;
                },
                getTranslationType: () => translationTypeRef.current,
            }),
            [translatedContentRef, translationTypeRef],
        ),
    );

    const supportDomain = useMemo(() => {
        if (
            translationType === TranslationApiType.DeepL ||
            translationType === TranslationApiType.GoogleWeb
        ) {
            return false;
        }

        return true;
    }, [translationType]);

    return (
        <>
            {/* 用表单处理下样式，但不用表单处理数据验证 */}
            <Form className="tool-translator-container" layout="vertical">
                <Flex gap={0} justify="space-between">
                    <Flex gap={0} align="center">
                        <Form.Item
                            style={{ marginBottom: token.marginXS }}
                            label={<FormattedMessage id="tools.translation.sourceLanguage" />}
                        >
                            <Select
                                value={sourceLanguage}
                                showSearch
                                onChange={setSourceLanguage}
                                options={[
                                    {
                                        label:
                                            intl.formatMessage({
                                                id: 'tools.translation.language.auto',
                                            }) +
                                            (autoLanguage
                                                ? ` (${languageCodeLabelMap.get(autoLanguage)})`
                                                : ''),
                                        title: intl.formatMessage({
                                            id: 'tools.translation.language.auto',
                                        }),
                                        value: 'auto',
                                    },
                                    ...(languageOptions ?? []),
                                ]}
                                variant="underlined"
                                styles={{
                                    popup: {
                                        root: {
                                            minWidth: 200,
                                        },
                                    },
                                }}
                                filterOption={selectFilterOption}
                            />
                        </Form.Item>
                        <Button
                            type="link"
                            disabled={
                                (sourceLanguage === 'auto' && !autoLanguage) ||
                                autoLanguage === targetLanguage ||
                                sourceLanguage === targetLanguage
                            }
                            icon={<SwapOutlined />}
                            style={{ marginTop: token.margin }}
                            onClick={() => {
                                let sourceValue = sourceLanguage;
                                if (sourceLanguage === 'auto') {
                                    if (!autoLanguage) {
                                        return;
                                    }
                                    sourceValue = autoLanguage;
                                }

                                setSourceLanguage(targetLanguage);
                                setTargetLanguage(sourceValue);
                            }}
                        />
                        <Form.Item
                            style={{ marginBottom: token.marginXS }}
                            label={<FormattedMessage id="tools.translation.targetLanguage" />}
                        >
                            <Select
                                showSearch
                                value={targetLanguage}
                                onChange={(value) => {
                                    setTargetLanguage(value);
                                    updateAppSettings(
                                        AppSettingsGroup.Cache,
                                        { targetLanguage: value },
                                        true,
                                        true,
                                        false,
                                    );
                                }}
                                options={languageOptions}
                                filterOption={selectFilterOption}
                                styles={{
                                    popup: {
                                        root: {
                                            minWidth: 200,
                                        },
                                    },
                                }}
                                variant="underlined"
                            />
                        </Form.Item>
                    </Flex>
                    <Flex gap={token.margin}>
                        <Form.Item
                            style={{ marginBottom: token.marginXS }}
                            label={<FormattedMessage id="tools.translation.type" />}
                        >
                            <Select
                                showSearch
                                value={translationType}
                                onChange={(value) => {
                                    setTranslationType(value);
                                    setAutoLanguage(undefined);
                                    updateAppSettings(
                                        AppSettingsGroup.Cache,
                                        { translationType: value },
                                        true,
                                        true,
                                        false,
                                    );
                                }}
                                options={supportedTranslationTypes.map((item) => ({
                                    label: (
                                        <ModelSelectLabel
                                            modelName={item.name}
                                            custom={
                                                typeof item.type === 'string' &&
                                                ('translationApiConfig' in item ||
                                                    'apiConfig' in item)
                                            }
                                            reasoner={
                                                typeof item.type === 'string' &&
                                                'apiConfig' in item &&
                                                item.apiConfig.support_thinking
                                            }
                                        />
                                    ),
                                    value: item.type,
                                }))}
                                loading={supportedTranslationTypesLoading}
                                filterOption={selectFilterOption}
                                styles={{
                                    popup: {
                                        root: {
                                            minWidth: 200,
                                        },
                                    },
                                }}
                                variant="underlined"
                            />
                        </Form.Item>
                        <Form.Item
                            style={{ marginBottom: token.marginXS }}
                            label={<FormattedMessage id="tools.translation.domain" />}
                            hidden={!supportDomain}
                        >
                            <Select
                                showSearch
                                value={translationDomain}
                                onChange={(value) => {
                                    setTranslationDomain(value);
                                    updateAppSettings(
                                        AppSettingsGroup.Cache,
                                        { translationDomain: value },
                                        true,
                                        true,
                                        false,
                                    );
                                }}
                                options={[
                                    {
                                        label: intl.formatMessage({
                                            id: 'tools.translation.domain.general',
                                        }),
                                        value: TranslationDomain.General,
                                    },
                                    {
                                        label: intl.formatMessage({
                                            id: 'tools.translation.domain.computers',
                                        }),
                                        value: TranslationDomain.Computers,
                                    },
                                    {
                                        label: intl.formatMessage({
                                            id: 'tools.translation.domain.medicine',
                                        }),
                                        value: TranslationDomain.Medicine,
                                    },
                                    {
                                        label: intl.formatMessage({
                                            id: 'tools.translation.domain.finance',
                                        }),
                                        value: TranslationDomain.Finance,
                                    },
                                    {
                                        label: intl.formatMessage({
                                            id: 'tools.translation.domain.game',
                                        }),
                                        value: TranslationDomain.Game,
                                    },
                                ]}
                                filterOption={selectFilterOption}
                                styles={{
                                    popup: {
                                        root: {
                                            minWidth: 200,
                                        },
                                    },
                                }}
                                variant="underlined"
                            />
                        </Form.Item>
                    </Flex>
                </Flex>
                <Row gutter={16} style={{ marginTop: token.marginXXS }}>
                    <Col span={12}>
                        <TextArea
                            ref={sourceContentRef}
                            rows={12}
                            disabled={disableInput}
                            maxLength={5000}
                            showCount
                            autoSize={{ minRows: 12 }}
                            placeholder={intl.formatMessage({
                                id: 'tools.translation.placeholder',
                            })}
                            value={sourceContent}
                            style={{ flex: 1 }}
                            onChange={(e) => setSourceContent(e.target.value)}
                        />
                    </Col>
                    <Col span={12}>
                        <Spin spinning={startLoading}>
                            <div style={{ position: 'relative' }}>
                                <Spin
                                    spinning={loading}
                                    style={{
                                        position: 'absolute',
                                        bottom: token.margin,
                                        right: token.marginLG,
                                    }}
                                />
                                <TextArea
                                    rows={12}
                                    variant="filled"
                                    style={{ flex: 1 }}
                                    autoSize={{ minRows: 12 }}
                                    readOnly
                                    value={translatedContent}
                                />
                            </div>
                        </Spin>
                    </Col>
                </Row>
            </Form>

            <style jsx>{`
                :global(.tool-translator-container .ant-form-item-label) {
                    padding-bottom: ${token.paddingXXS}px !important;
                }

                :global(.tool-translator-container .ant-form-item-label label) {
                    font-size: 12px !important;
                    color: ${token.colorTextDescription} !important;
                }
            `}</style>
        </>
    );
};

export const Translator = React.memo(TranslatorCore);

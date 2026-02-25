const path = require("path");
const HtmlWebpackPlugin = require("html-webpack-plugin");
const MiniCssExtractPlugin = require("mini-css-extract-plugin");

module.exports = (env, argv) => {
  const isDev = argv.mode === "development";
  return {
    entry: "./src/index.tsx",
    output: {
      path: path.resolve(__dirname, "dist"),
      filename: "bundle.[contenthash:8].js",
      clean: true,
    },
    resolve: {
      extensions: [".ts", ".tsx", ".js", ".jsx"],
      alias: { "@": path.resolve(__dirname, "src") },
    },
    module: {
      rules: [
        {
          test: /\.tsx?$/,
          use: "ts-loader",
          exclude: /node_modules/,
        },
        {
          test: /\.css$/,
          use: [
            isDev ? "style-loader" : MiniCssExtractPlugin.loader,
            "css-loader",
            "postcss-loader",
          ],
        },
      ],
    },
    plugins: [
      new HtmlWebpackPlugin({
        template: "./src/index.html",
      }),
      ...(!isDev ? [new MiniCssExtractPlugin({ filename: "styles.[contenthash:8].css" })] : []),
    ],
    devServer: {
      port: 1420,
      hot: true,
      headers: { "Access-Control-Allow-Origin": "*" },
    },
    devtool: isDev ? "eval-source-map" : false,
  };
};
